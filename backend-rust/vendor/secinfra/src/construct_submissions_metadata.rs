use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use arrow_array::builder::{BooleanBuilder, StringBuilder, UInt64Builder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use serde_json::Value;
use tracing::info;
use zip::ZipArchive;

use crate::common::sec_user_agent;

const SUBMISSIONS_ZIP_URL: &str =
    "https://www.sec.gov/Archives/edgar/daily-index/bulkdata/submissions.zip";
const DEFAULT_COLUMNS: [&str; 16] = [
    "accessionNumber",
    "filingDate",
    "reportDate",
    "acceptanceDateTime",
    "act",
    "form",
    "fileNumber",
    "filmNumber",
    "items",
    "core_type",
    "size",
    "isXBRL",
    "isInlineXBRL",
    "isXBRLNumeric",
    "primaryDocument",
    "primaryDocDescription",
];
const BATCH_TARGET_ROWS: usize = 100_000;
const WORKER_FILE_BATCH_SIZE: usize = 100;
const STRING_VALUE_BYTES_PER_ROW: usize = 32;
const ZSTD_COMPRESSION_LEVEL: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructSubmissionsMetadataStats {
    pub files_processed: usize,
    pub files_skipped: usize,
    pub filings_written: usize,
    pub batches_written: usize,
}

#[derive(Clone)]
struct ColumnSpec {
    name: String,
    kind: ColumnKind,
}

struct SubmissionBatchBuffer {
    ciks: Vec<u64>,
    columns: Vec<ColumnBuffer>,
    files: usize,
    json_bytes: usize,
    read_elapsed: Duration,
    parse_elapsed: Duration,
}

#[derive(Clone, Copy)]
enum ColumnKind {
    Utf8,
    UInt64,
    Boolean,
}

enum ColumnBuffer {
    Utf8(StringBuilder),
    UInt64(UInt64Builder),
    Boolean(BooleanBuilder),
}

struct ParquetWorkerResult {
    batch: RecordBatch,
    rows: usize,
    files: usize,
    json_bytes: usize,
    read_elapsed: Duration,
    parse_elapsed: Duration,
    build_elapsed: Duration,
}

struct TempZipFile {
    path: PathBuf,
}


pub async fn construct_submissions_metadata(
    output_path: impl Into<PathBuf>,
    submissions_zip_path: Option<PathBuf>,
    columns: Option<Vec<String>>,
    threads: Option<usize>,
) -> Result<ConstructSubmissionsMetadataStats> {
    let output_path = output_path.into();
    let columns = resolve_columns(columns)?;
    let threads = resolve_threads(threads)?;

    if let Some(submissions_zip_path) = submissions_zip_path {
        return tokio::task::spawn_blocking(move || {
            write_submissions_metadata_parquet_from_path(
                submissions_zip_path,
                output_path,
                columns,
                threads,
            )
        })
        .await?;
    }

    let temp_zip = download_submissions_zip_to_temp().await?;
    tokio::task::spawn_blocking(move || {
        write_submissions_metadata_parquet_from_path(&temp_zip.path, output_path, columns, threads)
    })
    .await?
}

pub fn construct_submissions_metadata_from_zip(
    output_path: impl AsRef<Path>,
    submissions_zip_path: impl AsRef<Path>,
    columns: Option<Vec<String>>,
    threads: Option<usize>,
) -> Result<ConstructSubmissionsMetadataStats> {
    let columns = resolve_columns(columns)?;
    let threads = resolve_threads(threads)?;

    write_submissions_metadata_parquet_from_path(
        submissions_zip_path.as_ref(),
        output_path,
        columns,
        threads,
    )
}

async fn download_submissions_zip_to_temp() -> Result<TempZipFile> {
    let client = reqwest::Client::builder()
        .user_agent(sec_user_agent())
        .build()
        .context("failed to build SEC HTTP client")?;

    info!(url = SUBMISSIONS_ZIP_URL, "Downloading SEC submissions zip");
    let bytes = client
        .get(SUBMISSIONS_ZIP_URL)
        .send()
        .await
        .context("failed to download SEC submissions zip")?
        .error_for_status()
        .context("SEC submissions zip request failed")?
        .bytes()
        .await
        .context("failed to read SEC submissions zip response")?
        .to_vec();
    let path = temp_submissions_zip_path()?;
    let bytes_len = bytes.len();
    tokio::task::spawn_blocking({
        let path = path.clone();
        move || -> Result<()> {
            let mut file = File::create(&path).with_context(|| {
                format!("failed to create temp submissions zip {}", path.display())
            })?;
            file.write_all(&bytes).with_context(|| {
                format!("failed to write temp submissions zip {}", path.display())
            })?;
            Ok(())
        }
    })
    .await??;
    info!(bytes = bytes_len, path = %path.display(), "Downloaded SEC submissions zip");
    Ok(TempZipFile { path })
}

fn write_submissions_metadata_parquet<R>(
    submissions_zip: R,
    output_path: impl AsRef<Path>,
    columns: Vec<ColumnSpec>,
) -> Result<ConstructSubmissionsMetadataStats>
where
    R: Read + Seek,
{
    let started_at = Instant::now();
    let mut archive =
        ZipArchive::new(submissions_zip).context("failed to open submissions zip archive")?;
    let schema = submissions_schema(&columns);
    let output = File::create(output_path.as_ref()).with_context(|| {
        format!(
            "failed to create submissions metadata parquet {}",
            output_path.as_ref().display()
        )
    })?;
    let props = parquet_writer_props()?;
    let mut writer = ArrowWriter::try_new(output, schema.clone(), Some(props))
        .context("failed to create parquet writer")?;
    let mut buffer = SubmissionBatchBuffer::new(&columns);
    let mut stats = ConstructSubmissionsMetadataStats {
        files_processed: 0,
        files_skipped: 0,
        filings_written: 0,
        batches_written: 0,
    };

    info!(
        zip_entries = archive.len(),
        columns = ?column_names(&columns),
        batch_target_rows = BATCH_TARGET_ROWS,
        compression = "zstd",
        zstd_level = ZSTD_COMPRESSION_LEVEL,
        output = "parquet",
        "Processing SEC submissions metadata zip"
    );

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .with_context(|| format!("failed to read submissions zip entry {index}"))?;

        let filename = file.name().to_string();
        if !filename.starts_with("CIK") {
            continue;
        }

        let rows = append_submission_file(&filename, &mut file, &columns, &mut buffer)
            .with_context(|| format!("failed to process submissions metadata file {filename}"))?;
        stats.files_processed += 1;
        stats.filings_written += rows;

        if buffer.len() >= BATCH_TARGET_ROWS {
            flush_parquet_batch(&mut writer, schema.clone(), &mut buffer, &mut stats)?;
        }
    }

    flush_parquet_batch(&mut writer, schema, &mut buffer, &mut stats)?;
    writer
        .close()
        .context("failed to close submissions metadata parquet writer")?;

    info!(
        files_processed = stats.files_processed,
        files_skipped = stats.files_skipped,
        filings_written = stats.filings_written,
        batches_written = stats.batches_written,
        total_ms = started_at.elapsed().as_millis(),
        output = "parquet",
        "Finished constructing submissions metadata"
    );

    Ok(stats)
}

fn write_submissions_metadata_parquet_from_path(
    submissions_zip_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    columns: Vec<ColumnSpec>,
    threads: usize,
) -> Result<ConstructSubmissionsMetadataStats> {
    if threads <= 1 {
        let file = File::open(submissions_zip_path.as_ref()).with_context(|| {
            format!(
                "failed to open submissions zip {}",
                submissions_zip_path.as_ref().display()
            )
        })?;
        return write_submissions_metadata_parquet(file, output_path, columns);
    }

    write_submissions_metadata_parquet_parallel(submissions_zip_path, output_path, columns, threads)
}

fn write_submissions_metadata_parquet_parallel(
    submissions_zip_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    columns: Vec<ColumnSpec>,
    threads: usize,
) -> Result<ConstructSubmissionsMetadataStats> {
    let started_at = Instant::now();
    let submissions_zip_path = submissions_zip_path.as_ref().to_path_buf();
    let filenames = collect_cik_filenames(&submissions_zip_path)?;
    let batches = distribute_filename_batches(&filenames, threads);
    let schema = submissions_schema(&columns);
    let output = File::create(output_path.as_ref()).with_context(|| {
        format!(
            "failed to create submissions metadata parquet {}",
            output_path.as_ref().display()
        )
    })?;
    let props = parquet_writer_props()?;
    let mut writer = ArrowWriter::try_new(output, schema.clone(), Some(props))
        .context("failed to create parquet writer")?;
    let mut stats = ConstructSubmissionsMetadataStats {
        files_processed: 0,
        files_skipped: 0,
        filings_written: 0,
        batches_written: 0,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel::<Result<ParquetWorkerResult>>(threads * 2);

    info!(
        zip_entries = filenames.len(),
        columns = ?column_names(&columns),
        batch_target_rows = BATCH_TARGET_ROWS,
        worker_file_batch_size = WORKER_FILE_BATCH_SIZE,
        threads,
        compression = "zstd",
        zstd_level = ZSTD_COMPRESSION_LEVEL,
        output = "parquet",
        "Processing SEC submissions metadata zip"
    );

    let scoped_result = thread::scope(|scope| -> Result<()> {
        for worker_batches in batches {
            let worker_tx = tx.clone();
            let worker_cancel = Arc::clone(&cancel);
            let worker_zip_path = submissions_zip_path.clone();
            let worker_columns = &columns;
            let worker_schema = schema.clone();
            scope.spawn(move || {
                process_parquet_worker_batches(
                    worker_zip_path,
                    worker_batches,
                    worker_columns,
                    worker_schema,
                    worker_tx,
                    worker_cancel,
                );
            });
        }
        drop(tx);

        let mut first_error = None;
        for result in rx {
            match result {
                Ok(result) => {
                    if first_error.is_some() {
                        continue;
                    }

                    let write_started_at = Instant::now();
                    if let Err(error) = writer
                        .write(&result.batch)
                        .context("failed to write submissions metadata parquet batch")
                    {
                        cancel.store(true, Ordering::Relaxed);
                        first_error = Some(error);
                        continue;
                    }
                    let parquet_write_ms = write_started_at.elapsed().as_millis();

                    stats.files_processed += result.files;
                    stats.filings_written += result.rows;
                    stats.batches_written += 1;
                    info!(
                        batch_index = stats.batches_written,
                        batch_rows = result.rows,
                        batch_files = result.files,
                        batch_json_bytes = result.json_bytes,
                        total_rows = stats.filings_written,
                        files_processed = stats.files_processed,
                        files_skipped = stats.files_skipped,
                        read_ms = result.read_elapsed.as_millis(),
                        parse_ms = result.parse_elapsed.as_millis(),
                        batch_build_ms = result.build_elapsed.as_millis(),
                        parquet_write_ms,
                        "Wrote submissions metadata parquet batch"
                    );
                }
                Err(error) => {
                    cancel.store(true, Ordering::Relaxed);
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(())
    });

    scoped_result?;
    writer
        .close()
        .context("failed to close submissions metadata parquet writer")?;

    info!(
        files_processed = stats.files_processed,
        files_skipped = stats.files_skipped,
        filings_written = stats.filings_written,
        batches_written = stats.batches_written,
        total_ms = started_at.elapsed().as_millis(),
        threads,
        output = "parquet",
        "Finished constructing submissions metadata"
    );

    Ok(stats)
}

fn collect_cik_filenames(submissions_zip_path: &Path) -> Result<Vec<String>> {
    let file = File::open(submissions_zip_path).with_context(|| {
        format!(
            "failed to open submissions zip {}",
            submissions_zip_path.display()
        )
    })?;
    let mut archive =
        ZipArchive::new(file).context("failed to open submissions zip archive")?;
    let mut filenames = Vec::new();

    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .with_context(|| format!("failed to read submissions zip entry {index}"))?;
        let filename = file.name();
        if filename.starts_with("CIK") {
            filenames.push(filename.to_string());
        }
    }

    Ok(filenames)
}

fn distribute_filename_batches(filenames: &[String], threads: usize) -> Vec<Vec<Vec<String>>> {
    let mut worker_batches = vec![Vec::new(); threads];
    for (batch_index, batch) in filenames.chunks(WORKER_FILE_BATCH_SIZE).enumerate() {
        worker_batches[batch_index % threads].push(batch.to_vec());
    }
    worker_batches
}

fn process_parquet_worker_batches(
    submissions_zip_path: PathBuf,
    batches: Vec<Vec<String>>,
    columns: &[ColumnSpec],
    schema: SchemaRef,
    tx: mpsc::SyncSender<Result<ParquetWorkerResult>>,
    cancel: Arc<AtomicBool>,
) {
    let result = process_parquet_worker_batches_inner(
        &submissions_zip_path,
        batches,
        columns,
        schema,
        &tx,
        &cancel,
    );
    if let Err(error) = result {
        cancel.store(true, Ordering::Relaxed);
        let _ = tx.send(Err(error));
    }
}

fn process_parquet_worker_batches_inner(
    submissions_zip_path: &Path,
    batches: Vec<Vec<String>>,
    columns: &[ColumnSpec],
    schema: SchemaRef,
    tx: &mpsc::SyncSender<Result<ParquetWorkerResult>>,
    cancel: &AtomicBool,
) -> Result<()> {
    let file = File::open(submissions_zip_path).with_context(|| {
        format!(
            "failed to open submissions zip {}",
            submissions_zip_path.display()
        )
    })?;
    let mut archive =
        ZipArchive::new(file).context("failed to open submissions zip archive")?;
    let mut buffer = SubmissionBatchBuffer::new(columns);

    for batch in batches {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        for filename in batch {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            let mut file = archive
                .by_name(&filename)
                .with_context(|| format!("failed to read submissions zip entry {filename}"))?;
            append_submission_file(&filename, &mut file, columns, &mut buffer)
                .with_context(|| {
                    format!("failed to process submissions metadata file {filename}")
                })?;

            if buffer.len() >= BATCH_TARGET_ROWS {
                send_parquet_worker_result(tx, schema.clone(), &mut buffer, cancel)?;
            }
        }
    }

    if !cancel.load(Ordering::Relaxed) && !buffer.is_empty() {
        send_parquet_worker_result(tx, schema, &mut buffer, cancel)?;
    }

    Ok(())
}

fn send_parquet_worker_result(
    tx: &mpsc::SyncSender<Result<ParquetWorkerResult>>,
    schema: SchemaRef,
    buffer: &mut SubmissionBatchBuffer,
    cancel: &AtomicBool,
) -> Result<()> {
    let rows = buffer.len();
    let files = buffer.files;
    let json_bytes = buffer.json_bytes;
    let read_elapsed = buffer.read_elapsed;
    let parse_elapsed = buffer.parse_elapsed;
    let build_started_at = Instant::now();
    let batch = buffer.take().into_record_batch(schema)?;
    let result = ParquetWorkerResult {
        batch,
        rows,
        files,
        json_bytes,
        read_elapsed,
        parse_elapsed,
        build_elapsed: build_started_at.elapsed(),
    };

    match tx.send(Ok(result)) {
        Ok(()) => Ok(()),
        Err(_) if cancel.load(Ordering::Relaxed) => Ok(()),
        Err(_) => Err(anyhow!("submissions metadata parquet writer channel closed")),
    }
}

fn append_submission_file<R>(
    filename: &str,
    mut reader: R,
    columns: &[ColumnSpec],
    buffer: &mut SubmissionBatchBuffer,
) -> Result<usize>
where
    R: Read,
{
    let read_started_at = Instant::now();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read submissions JSON {filename}"))?;
    let json_bytes = bytes.len();
    let read_elapsed = read_started_at.elapsed();

    append_submission_bytes(filename, &bytes, json_bytes, read_elapsed, columns, buffer)
}

fn append_submission_bytes(
    filename: &str,
    bytes: &[u8],
    json_bytes: usize,
    read_elapsed: Duration,
    columns: &[ColumnSpec],
    buffer: &mut SubmissionBatchBuffer,
) -> Result<usize> {
    let cik = cik_from_filename(filename)?;

    let parse_started_at = Instant::now();
    let data: Value = serde_json::from_slice(bytes)
        .with_context(|| format!("failed to parse submissions JSON {filename}"))?;
    let filings = filings_data(filename, &data)?;
    let row_count = field_array(filings, "accessionNumber")?.len();
    let column_arrays = columns
        .iter()
        .map(|column| field_array(filings, &column.name))
        .collect::<Result<Vec<_>>>()?;

    if column_arrays.iter().any(|values| values.len() < row_count) {
        return Err(anyhow!("submissions fields have mismatched lengths"));
    }

    buffer.reserve(row_count);
    for row_index in 0..row_count {
        buffer.ciks.push(cik);
        for (column_index, values) in column_arrays.iter().enumerate() {
            buffer.columns[column_index].push(&values[row_index]);
        }
    }
    buffer.files += 1;
    buffer.json_bytes += json_bytes;
    buffer.read_elapsed += read_elapsed;
    buffer.parse_elapsed += parse_started_at.elapsed();

    Ok(row_count)
}

fn flush_parquet_batch<W>(
    writer: &mut ArrowWriter<W>,
    schema: SchemaRef,
    buffer: &mut SubmissionBatchBuffer,
    stats: &mut ConstructSubmissionsMetadataStats,
) -> Result<()>
where
    W: std::io::Write + Send,
{
    if buffer.is_empty() {
        return Ok(());
    }

    let rows = buffer.len();
    let files = buffer.files;
    let json_bytes = buffer.json_bytes;
    let read_elapsed = buffer.read_elapsed;
    let parse_elapsed = buffer.parse_elapsed;
    let build_started_at = Instant::now();
    let batch = buffer.take().into_record_batch(schema)?;
    let batch_build_ms = build_started_at.elapsed().as_millis();

    let write_started_at = Instant::now();
    writer
        .write(&batch)
        .context("failed to write submissions metadata parquet batch")?;
    let parquet_write_ms = write_started_at.elapsed().as_millis();

    stats.batches_written += 1;
    info!(
        batch_index = stats.batches_written,
        batch_rows = rows,
        batch_files = files,
        batch_json_bytes = json_bytes,
        total_rows = stats.filings_written,
        files_processed = stats.files_processed,
        files_skipped = stats.files_skipped,
        read_ms = read_elapsed.as_millis(),
        parse_ms = parse_elapsed.as_millis(),
        batch_build_ms,
        parquet_write_ms,
        "Wrote submissions metadata parquet batch"
    );
    Ok(())
}

fn resolve_columns(columns: Option<Vec<String>>) -> Result<Vec<ColumnSpec>> {
    let columns = columns.unwrap_or_else(|| {
        DEFAULT_COLUMNS
            .iter()
            .map(|column| column.to_string())
            .collect()
    });
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(columns.len());

    if columns.is_empty() {
        return Err(anyhow!(
            "at least one submissions metadata column is required"
        ));
    }

    for column in &columns {
        if column.is_empty() {
            return Err(anyhow!("submissions metadata column name cannot be empty"));
        }
        if column == "cik" {
            return Err(anyhow!(
                "cik is always included and cannot be requested as a submissions metadata column"
            ));
        }
        if !seen.insert(column) {
            return Err(anyhow!("duplicate submissions metadata column {column}"));
        }
        resolved.push(ColumnSpec {
            name: column.clone(),
            kind: column_kind(column),
        });
    }

    Ok(resolved)
}

fn resolve_threads(threads: Option<usize>) -> Result<usize> {
    let threads = threads.unwrap_or(1);
    if threads == 0 {
        return Err(anyhow!("submissions metadata threads must be at least 1"));
    }
    Ok(threads)
}

fn temp_submissions_zip_path() -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before unix epoch")?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "secinfra-submissions-{}-{timestamp}.zip",
        std::process::id()
    )))
}

fn cik_from_filename(filename: &str) -> Result<u64> {
    filename
        .split('.')
        .next()
        .and_then(|stem| stem.split('-').next())
        .and_then(|stem| stem.strip_prefix("CIK"))
        .ok_or_else(|| anyhow!("filename does not start with CIK: {filename}"))?
        .parse::<u64>()
        .with_context(|| format!("failed to parse CIK from {filename}"))
}

impl Drop for TempZipFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn filings_data<'a>(filename: &str, data: &'a Value) -> Result<&'a Value> {
    if filename.contains("submissions") {
        return Ok(data);
    }

    object_get(data, "filings")
        .and_then(|filings| object_get(filings, "recent"))
        .ok_or_else(|| anyhow!("missing filings.recent"))
}

fn field_array<'a>(filings: &'a Value, field: &str) -> Result<&'a [Value]> {
    object_get(filings, field)
        .and_then(as_array)
        .ok_or_else(|| anyhow!("missing array field {field}"))
}

fn object_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(object) => object.get(key),
        _ => None,
    }
}

fn as_array(value: &Value) -> Option<&[Value]> {
    match value {
        Value::Array(values) => Some(values.as_slice()),
        _ => None,
    }
}

fn column_names(columns: &[ColumnSpec]) -> Vec<&str> {
    columns.iter().map(|column| column.name.as_str()).collect()
}

fn column_kind(column: &str) -> ColumnKind {
    match column {
        "size" => ColumnKind::UInt64,
        "isXBRL" | "isInlineXBRL" | "isXBRLNumeric" => ColumnKind::Boolean,
        _ => ColumnKind::Utf8,
    }
}

fn submissions_schema(columns: &[ColumnSpec]) -> SchemaRef {
    let mut fields = Vec::with_capacity(columns.len() + 1);
    fields.push(Field::new("cik", DataType::UInt64, false));
    fields.extend(columns.iter().map(|column| {
        Field::new(
            &column.name,
            match column.kind {
                ColumnKind::Utf8 => DataType::Utf8,
                ColumnKind::UInt64 => DataType::UInt64,
                ColumnKind::Boolean => DataType::Boolean,
            },
            false,
        )
    }));
    Arc::new(Schema::new(fields))
}

fn parquet_writer_props() -> Result<WriterProperties> {
    Ok(WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            ZstdLevel::try_new(ZSTD_COMPRESSION_LEVEL)
                .context("invalid submissions metadata parquet zstd level")?,
        ))
        .build())
}

fn append_metadata_value_string(builder: &mut StringBuilder, value: &Value) {
    match value {
        Value::Null => builder.append_value(""),
        Value::Bool(value) => builder.append_value(if *value { "true" } else { "false" }),
        Value::Number(value) => builder.append_value(value.to_string()),
        Value::String(value) => builder.append_value(value),
        Value::Array(_) | Value::Object(_) => builder.append_value(value.to_string()),
    }
}

fn metadata_value_u64(value: &Value) -> u64 {
    match value {
        Value::Number(value) => value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|value| value.try_into().ok()))
            .or_else(|| value.as_f64().map(|value| value as u64))
            .unwrap_or(0),
        Value::Bool(value) => u64::from(*value),
        Value::String(value) => value.parse().unwrap_or(0),
        Value::Null | Value::Array(_) | Value::Object(_) => 0,
    }
}

fn metadata_value_bool(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Number(value) => value
            .as_i64()
            .map(|value| value != 0)
            .or_else(|| value.as_u64().map(|value| value != 0))
            .or_else(|| value.as_f64().map(|value| value != 0.0))
            .unwrap_or(false),
        Value::String(value) => matches!(value.as_str(), "1" | "true" | "TRUE" | "True"),
        Value::Null | Value::Array(_) | Value::Object(_) => false,
    }
}

impl SubmissionBatchBuffer {
    fn new(columns: &[ColumnSpec]) -> Self {
        Self {
            ciks: Vec::with_capacity(BATCH_TARGET_ROWS),
            columns: columns
                .iter()
                .map(|column| ColumnBuffer::new(column.kind))
                .collect(),
            files: 0,
            json_bytes: 0,
            read_elapsed: Duration::ZERO,
            parse_elapsed: Duration::ZERO,
        }
    }

    fn len(&self) -> usize {
        self.ciks.len()
    }

    fn is_empty(&self) -> bool {
        self.ciks.is_empty()
    }

    fn reserve(&mut self, rows: usize) {
        self.ciks.reserve(rows);
        for column in &mut self.columns {
            column.reserve(rows);
        }
    }

    fn take(&mut self) -> Self {
        Self {
            ciks: std::mem::take(&mut self.ciks),
            columns: self.columns.iter_mut().map(ColumnBuffer::take).collect(),
            files: std::mem::take(&mut self.files),
            json_bytes: std::mem::take(&mut self.json_bytes),
            read_elapsed: std::mem::take(&mut self.read_elapsed),
            parse_elapsed: std::mem::take(&mut self.parse_elapsed),
        }
    }

    fn into_record_batch(self, schema: SchemaRef) -> Result<RecordBatch> {
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(self.columns.len() + 1);
        let mut cik_builder = UInt64Builder::with_capacity(self.ciks.len());
        for cik in self.ciks {
            cik_builder.append_value(cik);
        }
        arrays.push(Arc::new(cik_builder.finish()));
        arrays.extend(self.columns.into_iter().map(ColumnBuffer::into_array));

        RecordBatch::try_new(schema, arrays)
            .context("failed to build submissions metadata record batch")
    }
}

impl ColumnBuffer {
    fn new(kind: ColumnKind) -> Self {
        match kind {
            ColumnKind::Utf8 => Self::Utf8(string_builder()),
            ColumnKind::UInt64 => Self::UInt64(UInt64Builder::with_capacity(BATCH_TARGET_ROWS)),
            ColumnKind::Boolean => Self::Boolean(BooleanBuilder::with_capacity(BATCH_TARGET_ROWS)),
        }
    }

    fn reserve(&mut self, rows: usize) {
        match self {
            Self::Utf8(_) | Self::UInt64(_) | Self::Boolean(_) => {
                let _ = rows;
            }
        }
    }

    fn push(&mut self, value: &Value) {
        match self {
            Self::Utf8(values) => append_metadata_value_string(values, value),
            Self::UInt64(values) => values.append_value(metadata_value_u64(value)),
            Self::Boolean(values) => values.append_value(metadata_value_bool(value)),
        }
    }

    fn into_array(mut self) -> ArrayRef {
        match &mut self {
            Self::Utf8(values) => Arc::new(values.finish()),
            Self::UInt64(values) => Arc::new(values.finish()),
            Self::Boolean(values) => Arc::new(values.finish()),
        }
    }

    fn take(&mut self) -> Self {
        match self {
            Self::Utf8(values) => {
                let old = std::mem::replace(values, string_builder());
                Self::Utf8(old)
            }
            Self::UInt64(values) => {
                let old =
                    std::mem::replace(values, UInt64Builder::with_capacity(BATCH_TARGET_ROWS));
                Self::UInt64(old)
            }
            Self::Boolean(values) => {
                let old =
                    std::mem::replace(values, BooleanBuilder::with_capacity(BATCH_TARGET_ROWS));
                Self::Boolean(old)
            }
        }
    }
}

fn string_builder() -> StringBuilder {
    StringBuilder::with_capacity(
        BATCH_TARGET_ROWS,
        BATCH_TARGET_ROWS * STRING_VALUE_BYTES_PER_ROW,
    )
}
