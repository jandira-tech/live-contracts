use crate::{
    CompressionType, ParsedSgml, ParsedSubmissionMetadata, SubmissionEvent, SubmissionEventType,
    compress_bytes,
};
use anyhow::{Result, anyhow};
use std::collections::HashSet;

const RANGE_WIDTH: usize = 20;
const RANGE_PLACEHOLDER: &str = "99999999999999999999";

#[derive(Debug)]
pub struct FilingTarArchive {
    pub bytes: Vec<u8>,
    pub document_count: usize,
    pub uuencoded_count: usize,
    pub document_ranges: Vec<DocumentByteRange>,
    pub submission_type: Option<String>,
    pub filing_date: Option<String>,
    pub report_date: Option<String>,
    pub contains_xbrl: bool,
    pub ciks: Vec<u64>,
    pub documents: Vec<FilingDocument>,
}

#[derive(Debug)]
pub struct DocumentByteRange {
    pub filename: String,
    pub start_byte: String,
    pub end_byte: String,
}

#[derive(Debug, Clone)]
pub struct FilingDocument {
    pub sequence: u16,
    pub document_type: String,
    pub filename: String,
    pub description: String,
    pub tar_start_byte: u64,
    pub tar_end_byte: u64,
}

struct PreparedDocument {
    sequence_number: u16,
    doc_type: Vec<u8>,
    sequence: Vec<u8>,
    filename: Vec<u8>,
    description: Vec<u8>,
    tar_filename: String,
    compressed_content: Vec<u8>,
    start_byte: String,
    end_byte: String,
}

pub fn build_tar_from_sgml(sgml: &[u8], zstd_level: i32) -> Result<FilingTarArchive> {
    let parsed = ParsedSgml::parse(sgml)?;
    let submission_metadata = ParsedSubmissionMetadata::parse(sgml)?;
    let submission_events = submission_metadata.events();
    let stats = parsed.stats();
    let document_count = stats.doc_count;
    let uuencoded_count = stats.uuencoded_count;
    let submission_type = metadata_value(&submission_events, "type");
    let filing_date = metadata_value(&submission_events, "filing-date")
        .and_then(|date| normalize_sec_date(&date));
    let report_date =
        metadata_value(&submission_events, "period").and_then(|date| normalize_sec_date(&date));
    let ciks = collect_ciks(&submission_events);

    let mut filenames = HashSet::new();
    filenames.insert("metadata.json".to_string());
    let mut documents = Vec::new();

    for (index, doc) in parsed.documents().enumerate() {
        let compressed_content = compress_bytes(CompressionType::Zstd, zstd_level, doc.content())?;
        let sequence_number = parse_sequence(doc.sequence()).unwrap_or((index + 1) as u16);
        let tar_filename = unique_filename(
            format!("{}.zst", sanitize_filename(doc.filename(), index + 1)),
            &mut filenames,
        );

        documents.push(PreparedDocument {
            sequence_number,
            doc_type: doc.doc_type().to_vec(),
            sequence: doc.sequence().to_vec(),
            filename: doc.filename().to_vec(),
            description: doc.description().to_vec(),
            tar_filename,
            compressed_content,
            start_byte: RANGE_PLACEHOLDER.to_string(),
            end_byte: RANGE_PLACEHOLDER.to_string(),
        });
    }

    let metadata_placeholder = metadata_json(&submission_events, &documents);
    let mut current_pos = next_tar_entry_offset(512 + metadata_placeholder.len());
    for doc in &mut documents {
        let start_byte = current_pos + 512;
        let end_byte = start_byte + doc.compressed_content.len();
        doc.start_byte = format_fixed_range(start_byte);
        doc.end_byte = format_fixed_range(end_byte);
        current_pos = next_tar_entry_offset(end_byte);
    }

    let metadata = metadata_json(&submission_events, &documents);
    if metadata.len() != metadata_placeholder.len() {
        return Err(anyhow!(
            "metadata size changed after byte range insertion: {} -> {}",
            metadata_placeholder.len(),
            metadata.len()
        ));
    }

    let mut tar_bytes = Vec::new();
    append_tar_entry(&mut tar_bytes, "metadata.json", &metadata)?;

    let mut document_ranges = Vec::with_capacity(documents.len());
    let mut filing_documents = Vec::with_capacity(documents.len());
    for doc in &documents {
        append_tar_entry(&mut tar_bytes, &doc.tar_filename, &doc.compressed_content)?;
        document_ranges.push(DocumentByteRange {
            filename: String::from_utf8_lossy(&doc.filename).into_owned(),
            start_byte: doc.start_byte.clone(),
            end_byte: doc.end_byte.clone(),
        });
        filing_documents.push(FilingDocument {
            sequence: doc.sequence_number,
            document_type: String::from_utf8_lossy(&doc.doc_type).into_owned(),
            filename: String::from_utf8_lossy(&doc.filename).into_owned(),
            description: String::from_utf8_lossy(&doc.description).into_owned(),
            tar_start_byte: doc.start_byte.parse().unwrap_or(0),
            tar_end_byte: doc.end_byte.parse().unwrap_or(0),
        });
    }

    finish_tar(&mut tar_bytes);
    let contains_xbrl = filing_documents.iter().any(document_looks_like_xbrl);

    Ok(FilingTarArchive {
        bytes: tar_bytes,
        document_count,
        uuencoded_count,
        document_ranges,
        submission_type,
        filing_date,
        report_date,
        contains_xbrl,
        ciks,
        documents: filing_documents,
    })
}

fn append_tar_entry(tar: &mut Vec<u8>, path: &str, bytes: &[u8]) -> Result<()> {
    if path.len() > 100 {
        return Err(anyhow!("tar entry path too long: {path}"));
    }
    if bytes.len() as u64 > 0o77777777777 {
        return Err(anyhow!("tar entry too large: {path}"));
    }

    let mut header = [0u8; 512];
    header[0..path.len()].copy_from_slice(path.as_bytes());
    write_tar_octal(&mut header[100..108], 0o644);
    write_tar_octal(&mut header[108..116], 0);
    write_tar_octal(&mut header[116..124], 0);
    write_tar_octal(&mut header[124..136], bytes.len() as u64);
    write_tar_octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");

    let checksum: u32 = header.iter().map(|byte| *byte as u32).sum();
    write_tar_checksum(&mut header[148..156], checksum);

    tar.extend_from_slice(&header);
    tar.extend_from_slice(bytes);

    let padding = (512 - (bytes.len() % 512)) % 512;
    tar.resize(tar.len() + padding, 0);

    Ok(())
}

fn finish_tar(tar: &mut Vec<u8>) {
    tar.resize(tar.len() + 1024, 0);
}

fn write_tar_octal(field: &mut [u8], value: u64) {
    field.fill(b'0');
    let octal = format!("{value:o}");
    let start = field.len().saturating_sub(octal.len() + 1);
    field[start..start + octal.len()].copy_from_slice(octal.as_bytes());
    field[field.len() - 1] = 0;
}

fn write_tar_checksum(field: &mut [u8], checksum: u32) {
    field.fill(b' ');
    let octal = format!("{checksum:06o}");
    field[..octal.len()].copy_from_slice(octal.as_bytes());
    field[6] = 0;
}

fn next_tar_entry_offset(data_end: usize) -> usize {
    data_end + tar_padding_len(data_end)
}

fn tar_padding_len(position: usize) -> usize {
    (512 - (position % 512)) % 512
}

fn format_fixed_range(value: usize) -> String {
    format!("{value:0width$}", width = RANGE_WIDTH)
}

fn metadata_value(events: &[SubmissionEvent], key: &str) -> Option<String> {
    events
        .iter()
        .find(|event| {
            event.event_type == SubmissionEventType::KeyValue
                && event.key.as_slice() == key.as_bytes()
        })
        .map(|event| String::from_utf8_lossy(&event.value).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_sec_date(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() == 8 {
        Some(format!(
            "{}-{}-{}",
            &digits[0..4],
            &digits[4..6],
            &digits[6..8]
        ))
    } else if raw.len() == 10 {
        Some(raw.to_string())
    } else {
        None
    }
}

fn parse_sequence(bytes: &[u8]) -> Option<u16> {
    std::str::from_utf8(bytes).ok()?.trim().parse().ok()
}

fn collect_ciks(events: &[SubmissionEvent]) -> Vec<u64> {
    let mut seen = HashSet::new();
    let mut ciks = Vec::new();
    for event in events {
        if event.event_type == SubmissionEventType::KeyValue
            && event.key.eq_ignore_ascii_case(b"cik")
        {
            let value = String::from_utf8_lossy(&event.value);
            if let Ok(cik) = value.trim().parse::<u64>() {
                if seen.insert(cik) {
                    ciks.push(cik);
                }
            }
        }
    }
    ciks
}

fn document_looks_like_xbrl(doc: &FilingDocument) -> bool {
    let document_type = doc.document_type.to_ascii_uppercase();
    let filename = doc.filename.to_ascii_lowercase();

    document_type.starts_with("EX-101")
        || document_type == "XML"
        || filename.ends_with(".xbrl")
        || filename.ends_with(".xml")
        || filename.ends_with(".xsd")
        || filename.ends_with(".cal")
        || filename.ends_with(".def")
        || filename.ends_with(".lab")
        || filename.ends_with(".pre")
}

fn metadata_json(events: &[SubmissionEvent], documents: &[PreparedDocument]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(b'{');
    let wrote_metadata = write_json_object_members(&mut out, events, 0, events.len(), -1);
    if wrote_metadata {
        out.push(b',');
    }
    out.extend_from_slice(br#""documents":["#);
    for (idx, doc) in documents.iter().enumerate() {
        if idx > 0 {
            out.push(b',');
        }
        write_document_metadata_json(&mut out, doc);
    }
    out.extend_from_slice(b"]}");
    out.push(b'\n');
    out
}

fn write_document_metadata_json(out: &mut Vec<u8>, doc: &PreparedDocument) {
    out.push(b'{');
    out.extend_from_slice(br#""type":"#);
    write_json_string(out, &doc.doc_type);
    out.extend_from_slice(br#","sequence":"#);
    write_json_string(out, &doc.sequence);
    out.extend_from_slice(br#","filename":"#);
    write_json_string(out, &doc.filename);
    out.extend_from_slice(br#","description":"#);
    write_json_string(out, &doc.description);
    out.extend_from_slice(br#","secsgml_start_byte":"#);
    write_json_string(out, doc.start_byte.as_bytes());
    out.extend_from_slice(br#","secsgml_end_byte":"#);
    write_json_string(out, doc.end_byte.as_bytes());
    out.push(b'}');
}

struct JsonMember {
    idx: usize,
    end_idx: usize,
}

fn write_json_object_range(
    out: &mut Vec<u8>,
    events: &[SubmissionEvent],
    start_idx: usize,
    end_idx: usize,
    depth: i32,
) {
    out.push(b'{');
    write_json_object_members(out, events, start_idx, end_idx, depth);
    out.push(b'}');
}

fn write_json_object_members(
    out: &mut Vec<u8>,
    events: &[SubmissionEvent],
    start_idx: usize,
    end_idx: usize,
    depth: i32,
) -> bool {
    let mut members = Vec::new();
    let mut i = start_idx;
    while i < end_idx {
        if events[i].depth != depth + 1 {
            i += 1;
            continue;
        }

        match events[i].event_type {
            SubmissionEventType::KeyValue => {
                members.push(JsonMember { idx: i, end_idx: i });
                i += 1;
            }
            SubmissionEventType::SectionStart => {
                let section_end = find_section_end(events, end_idx, i);
                members.push(JsonMember {
                    idx: i,
                    end_idx: section_end,
                });
                i = section_end.saturating_add(1);
            }
            _ => i += 1,
        }
    }

    let mut first = true;
    let mut emitted_keys: Vec<&[u8]> = Vec::new();

    for member in &members {
        let key = events[member.idx].key.as_slice();
        if emitted_keys.iter().any(|emitted| *emitted == key) {
            continue;
        }

        if !first {
            out.push(b',');
        }
        first = false;

        write_json_string(out, key);
        out.push(b':');

        let matching: Vec<&JsonMember> = members
            .iter()
            .filter(|candidate| events[candidate.idx].key.as_slice() == key)
            .collect();

        if matching.len() > 1 {
            out.push(b'[');
            for (idx, matching_member) in matching.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                write_json_member_value(out, events, matching_member);
            }
            out.push(b']');
        } else {
            write_json_member_value(out, events, member);
        }

        emitted_keys.push(key);
    }

    !first
}

fn find_section_end(events: &[SubmissionEvent], end_idx: usize, start_idx: usize) -> usize {
    let depth = events[start_idx].depth;
    for (idx, event) in events.iter().enumerate().take(end_idx).skip(start_idx + 1) {
        if event.event_type == SubmissionEventType::SectionEnd && event.depth == depth {
            return idx;
        }
    }

    end_idx
}

fn write_json_member_value(out: &mut Vec<u8>, events: &[SubmissionEvent], member: &JsonMember) {
    match events[member.idx].event_type {
        SubmissionEventType::KeyValue => write_json_string(out, &events[member.idx].value),
        SubmissionEventType::SectionStart => {
            write_json_object_range(
                out,
                events,
                member.idx + 1,
                member.end_idx,
                events[member.idx].depth,
            );
        }
        _ => out.extend_from_slice(b"null"),
    }
}

fn write_json_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(b'"');
    for &byte in bytes {
        match byte {
            b'"' => out.extend_from_slice(br#"\""#),
            b'\\' => out.extend_from_slice(br#"\\"#),
            0x20..=0x7f => out.push(byte),
            _ => out.extend_from_slice(format!("\\u{byte:04X}").as_bytes()),
        }
    }
    out.push(b'"');
}

fn sanitize_filename(name: &[u8], index: usize) -> String {
    let mut sanitized = String::new();

    for &byte in name {
        let ch = match byte {
            b'\\' | b'/' | b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|' => '_',
            0x20..=0x7e => byte as char,
            _ => '_',
        };
        sanitized.push(ch);
    }

    let sanitized = sanitized.trim().trim_matches('.').to_string();
    if sanitized.is_empty() {
        format!("doc_{index}.bin")
    } else {
        sanitized
    }
}

fn unique_filename(filename: String, seen: &mut HashSet<String>) -> String {
    for count in 1usize.. {
        let candidate = if count == 1 {
            fit_tar_filename(&filename, "")
        } else {
            fit_tar_filename(&filename, &format!("-{count}"))
        };

        if seen.insert(candidate.clone()) {
            return candidate;
        }
    }

    unreachable!("unbounded filename suffix loop")
}

fn fit_tar_filename(filename: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        if let Some((stem, ext)) = filename.rsplit_once('.') {
            let ext = format!(".{ext}");
            if !stem.is_empty() && ext.len() < 100 {
                let stem_len = 100 - ext.len();
                let stem: String = stem.chars().take(stem_len).collect();
                return format!("{stem}{ext}");
            }
        }
        return filename.chars().take(100).collect();
    }

    if let Some((stem, ext)) = filename.rsplit_once('.') {
        let ext = format!(".{ext}");
        if !stem.is_empty() && ext.len() <= 20 && suffix.len() + ext.len() < 100 {
            let stem_len = 100 - suffix.len() - ext.len();
            let stem: String = stem.chars().take(stem_len).collect();
            return format!("{stem}{suffix}{ext}");
        }
    }

    let name_len = 100usize.saturating_sub(suffix.len());
    let name: String = filename.chars().take(name_len).collect();
    format!("{name}{suffix}")
}
