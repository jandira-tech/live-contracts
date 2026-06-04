use bytes::Bytes;
use futures::{Stream, StreamExt};
use std::{
    io::{self, Write},
    pin::Pin,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    Zstd,
}

#[derive(Debug)]
pub struct CompressedBytes {
    pub bytes: Vec<u8>,
    pub input_bytes: usize,
    pub compressed_bytes: usize,
}

#[derive(Debug)]
pub struct CompressedStreamStats {
    pub input_bytes: usize,
    pub compressed_bytes: usize,
}

pub type IoByteStream = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send>>;

pub struct CompressedByteStream {
    pub stream: IoByteStream,
    pub completion: JoinHandle<anyhow::Result<CompressedStreamStats>>,
}

pub fn compress_bytes(
    compression_type: CompressionType,
    level: i32,
    bytes: &[u8],
) -> anyhow::Result<Vec<u8>> {
    match compression_type {
        CompressionType::Zstd => zstd::bulk::compress(bytes, level).map_err(Into::into),
    }
}

pub async fn compress_bytes_async(
    compression_type: CompressionType,
    level: i32,
    bytes: Bytes,
) -> anyhow::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || compress_bytes(compression_type, level, &bytes)).await?
}

pub async fn compress_byte_stream_async<S, E>(
    compression_type: CompressionType,
    level: i32,
    threads: Option<u32>,
    stream: S,
) -> anyhow::Result<CompressedBytes>
where
    S: Stream<Item = Result<Bytes, E>> + Send,
    E: Into<anyhow::Error> + Send + Sync + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Bytes>(4);
    let compression_task = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        match compression_type {
            CompressionType::Zstd => {
                let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), level)?;
                if let Some(threads) = threads {
                    encoder.multithread(threads)?;
                }
                while let Some(chunk) = rx.blocking_recv() {
                    encoder.write_all(&chunk)?;
                }
                encoder.finish().map_err(Into::into)
            }
        }
    });

    let mut input_bytes = 0usize;
    futures::pin_mut!(stream);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(Into::into)?;
        input_bytes += chunk.len();
        tx.send(chunk)
            .await
            .map_err(|_| anyhow::anyhow!("compression task stopped before stream ended"))?;
    }
    drop(tx);

    let bytes = compression_task.await??;
    let compressed_bytes = bytes.len();

    Ok(CompressedBytes {
        bytes,
        input_bytes,
        compressed_bytes,
    })
}

pub fn compress_byte_stream_to_stream<S, E>(
    compression_type: CompressionType,
    level: i32,
    threads: Option<u32>,
    stream: S,
) -> CompressedByteStream
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Into<anyhow::Error> + Send + Sync + 'static,
{
    let (raw_tx, mut raw_rx) = mpsc::channel::<Bytes>(4);
    let (compressed_tx, mut compressed_rx) = mpsc::channel::<io::Result<Bytes>>(4);
    let input_error_tx = compressed_tx.clone();

    let input_task = tokio::spawn(async move {
        let mut input_bytes = 0usize;
        futures::pin_mut!(stream);

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    let err = err.into();
                    let message = err.to_string();
                    let _ = input_error_tx.send(Err(io_error(message))).await;
                    return Err(err);
                }
            };

            input_bytes += chunk.len();
            raw_tx
                .send(chunk)
                .await
                .map_err(|_| anyhow::anyhow!("compression task stopped before stream ended"))?;
        }

        Ok(input_bytes)
    });

    let compression_error_tx = compressed_tx.clone();
    let compression_task = tokio::task::spawn_blocking(move || {
        let result = compress_raw_receiver_to_channel(
            compression_type,
            level,
            threads,
            &mut raw_rx,
            compressed_tx,
        );

        if let Err(err) = &result {
            let _ = compression_error_tx.blocking_send(Err(io_error(err.to_string())));
        }

        result
    });

    let completion = tokio::spawn(async move {
        let input_result = input_task.await;
        let compression_result = compression_task.await;

        let input_bytes = input_result??;
        let compressed_bytes = compression_result??;

        Ok(CompressedStreamStats {
            input_bytes,
            compressed_bytes,
        })
    });

    let stream = async_stream::try_stream! {
        while let Some(chunk) = compressed_rx.recv().await {
            yield chunk?;
        }
    };

    CompressedByteStream {
        stream: Box::pin(stream),
        completion,
    }
}

fn compress_raw_receiver_to_channel(
    compression_type: CompressionType,
    level: i32,
    threads: Option<u32>,
    raw_rx: &mut mpsc::Receiver<Bytes>,
    compressed_tx: mpsc::Sender<io::Result<Bytes>>,
) -> anyhow::Result<usize> {
    match compression_type {
        CompressionType::Zstd => {
            let writer = ChannelWriter::new(compressed_tx, 1024 * 1024);
            let mut encoder = zstd::stream::write::Encoder::new(writer, level)?;
            if let Some(threads) = threads {
                encoder.multithread(threads)?;
            }

            while let Some(chunk) = raw_rx.blocking_recv() {
                encoder.write_all(&chunk)?;
            }

            let writer = encoder.finish()?;
            writer.finish().map_err(Into::into)
        }
    }
}

struct ChannelWriter {
    tx: mpsc::Sender<io::Result<Bytes>>,
    buffer: Vec<u8>,
    chunk_size: usize,
    compressed_bytes: usize,
}

impl ChannelWriter {
    fn new(tx: mpsc::Sender<io::Result<Bytes>>, chunk_size: usize) -> Self {
        Self {
            tx,
            buffer: Vec::with_capacity(chunk_size),
            chunk_size,
            compressed_bytes: 0,
        }
    }

    fn finish(mut self) -> io::Result<usize> {
        self.send_buffer()?;
        Ok(self.compressed_bytes)
    }

    fn send_buffer(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let chunk = std::mem::take(&mut self.buffer);
        self.compressed_bytes += chunk.len();
        self.tx
            .blocking_send(Ok(Bytes::from(chunk)))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "compressed stream closed"))
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);

        while self.buffer.len() >= self.chunk_size {
            let rest = self.buffer.split_off(self.chunk_size);
            let chunk = std::mem::replace(&mut self.buffer, rest);
            self.compressed_bytes += chunk.len();
            self.tx.blocking_send(Ok(Bytes::from(chunk))).map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "compressed stream closed")
            })?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn io_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message.into())
}
