use std::{marker::PhantomData, os::raw::c_int, ptr::NonNull, slice};

#[repr(C)]
#[derive(Clone, Copy)]
struct ByteSpan {
    ptr: *const u8,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DocumentMeta {
    doc_type: ByteSpan,
    sequence: ByteSpan,
    filename: ByteSpan,
    description: ByteSpan,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawDocument {
    meta: DocumentMeta,
    content_start: *const u8,
    content_len: usize,
    decoded: *mut u8,
    decoded_len: usize,
    is_uuencoded: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SgmlStatus {
    Ok = 0,
    Oom = 1,
    Truncated = 2,
}

#[repr(C)]
struct RawParseResult {
    docs: *mut RawDocument,
    doc_count: usize,
    doc_cap: usize,
    status: SgmlStatus,
}

#[repr(C)]
#[derive(Default)]
struct RawParseStats {
    doc_count: usize,
    uuencoded_count: usize,
}

#[repr(C)]
struct RawSubmissionEvent {
    event_type: c_int,
    key: ByteSpan,
    value: ByteSpan,
    depth: c_int,
}

#[repr(C)]
struct RawSubmissionMetadata {
    events: *mut RawSubmissionEvent,
    count: usize,
    cap: usize,
    status: SgmlStatus,
}

#[repr(C)]
struct RawStandardizedSubmissionMetadata {
    events: *mut RawSubmissionEvent,
    count: usize,
    cap: usize,
    arena: *mut u8,
    arena_len: usize,
    arena_cap: usize,
    status: SgmlStatus,
}

unsafe extern "C" {
    fn parse_sgml(buf: *const u8, len: usize, stats: *mut RawParseStats) -> RawParseResult;
    fn free_sgml_parse_result(result: *mut RawParseResult);
    fn parse_submission_metadata(buf: *const u8, len: usize) -> RawSubmissionMetadata;
    fn free_submission_metadata(metadata: *mut RawSubmissionMetadata);
    fn standardize_submission_metadata(
        metadata: *const RawSubmissionMetadata,
    ) -> RawStandardizedSubmissionMetadata;
    fn free_standardized_submission_metadata(metadata: *mut RawStandardizedSubmissionMetadata);
}

#[derive(Debug)]
pub struct ParseStats {
    pub doc_count: usize,
    pub uuencoded_count: usize,
}

#[derive(Debug)]
pub struct ParseError {
    status: SgmlStatus,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "secsgmlc parse failed with status {:?}", self.status)
    }
}

impl std::error::Error for ParseError {}

pub struct ParsedSgml<'a> {
    result: RawParseResult,
    stats: ParseStats,
    _sgml: PhantomData<&'a [u8]>,
}

pub struct ParsedSubmissionMetadata<'a> {
    raw: RawSubmissionMetadata,
    standardized: RawStandardizedSubmissionMetadata,
    _sgml: PhantomData<&'a [u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionEventType {
    SectionStart,
    SectionEnd,
    KeyValue,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct SubmissionEvent {
    pub event_type: SubmissionEventType,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub depth: i32,
}

impl<'a> ParsedSgml<'a> {
    pub fn parse(sgml: &'a [u8]) -> Result<Self, ParseError> {
        let mut raw_stats = RawParseStats::default();
        let result = unsafe { parse_sgml(sgml.as_ptr(), sgml.len(), &mut raw_stats) };

        if result.status != SgmlStatus::Ok {
            let status = result.status;
            let mut result = result;
            unsafe { free_sgml_parse_result(&mut result) };
            return Err(ParseError { status });
        }

        Ok(Self {
            result,
            stats: ParseStats {
                doc_count: raw_stats.doc_count,
                uuencoded_count: raw_stats.uuencoded_count,
            },
            _sgml: PhantomData,
        })
    }

    pub fn stats(&self) -> &ParseStats {
        &self.stats
    }

    pub fn documents(&self) -> Documents<'_> {
        let docs = if self.result.docs.is_null() || self.result.doc_count == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(self.result.docs, self.result.doc_count) }
        };

        Documents { iter: docs.iter() }
    }
}

impl<'a> ParsedSubmissionMetadata<'a> {
    pub fn parse(sgml: &'a [u8]) -> Result<Self, ParseError> {
        let mut raw = unsafe { parse_submission_metadata(sgml.as_ptr(), sgml.len()) };
        if raw.status != SgmlStatus::Ok {
            let status = raw.status;
            unsafe { free_submission_metadata(&mut raw) };
            return Err(ParseError { status });
        }

        let mut standardized = unsafe { standardize_submission_metadata(&raw) };
        if standardized.status != SgmlStatus::Ok {
            let status = standardized.status;
            unsafe {
                free_standardized_submission_metadata(&mut standardized);
                free_submission_metadata(&mut raw);
            }
            return Err(ParseError { status });
        }

        Ok(Self {
            raw,
            standardized,
            _sgml: PhantomData,
        })
    }

    pub fn events(&self) -> Vec<SubmissionEvent> {
        let events = if self.standardized.events.is_null() || self.standardized.count == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(self.standardized.events, self.standardized.count) }
        };

        events
            .iter()
            .map(|event| SubmissionEvent {
                event_type: match event.event_type {
                    1 => SubmissionEventType::SectionStart,
                    2 => SubmissionEventType::SectionEnd,
                    3 => SubmissionEventType::KeyValue,
                    _ => SubmissionEventType::Unknown,
                },
                key: span_bytes(event.key).to_vec(),
                value: span_bytes(event.value).to_vec(),
                depth: event.depth,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.standardized.count == 0
    }
}

impl Drop for ParsedSubmissionMetadata<'_> {
    fn drop(&mut self) {
        unsafe {
            free_standardized_submission_metadata(&mut self.standardized);
            free_submission_metadata(&mut self.raw);
        }
    }
}

impl Drop for ParsedSgml<'_> {
    fn drop(&mut self) {
        unsafe { free_sgml_parse_result(&mut self.result) };
    }
}

pub struct Documents<'a> {
    iter: slice::Iter<'a, RawDocument>,
}

impl<'a> Iterator for Documents<'a> {
    type Item = DocumentRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|doc| DocumentRef { doc })
    }
}

pub struct DocumentRef<'a> {
    doc: &'a RawDocument,
}

impl<'a> DocumentRef<'a> {
    pub fn doc_type(&self) -> &'a [u8] {
        span_bytes(self.doc.meta.doc_type)
    }

    pub fn sequence(&self) -> &'a [u8] {
        span_bytes(self.doc.meta.sequence)
    }

    pub fn filename(&self) -> &'a [u8] {
        span_bytes(self.doc.meta.filename)
    }

    pub fn description(&self) -> &'a [u8] {
        span_bytes(self.doc.meta.description)
    }

    pub fn content(&self) -> &'a [u8] {
        if self.doc.is_uuencoded != 0 {
            ptr_bytes(self.doc.decoded.cast_const(), self.doc.decoded_len)
        } else {
            ptr_bytes(self.doc.content_start, self.doc.content_len)
        }
    }
}

fn span_bytes<'a>(span: ByteSpan) -> &'a [u8] {
    ptr_bytes(span.ptr, span.len)
}

fn ptr_bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    match (NonNull::new(ptr.cast_mut()), len) {
        (_, 0) => &[],
        (Some(ptr), len) => unsafe { slice::from_raw_parts(ptr.as_ptr(), len) },
        (None, _) => &[],
    }
}
