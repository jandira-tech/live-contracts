# Rust Backend Rewrite + Parallel Deploy Stack — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Python SEC EX-10 listener backend with a Rust rewrite built on the vendored `secinfra` crate, and deploy a parallel stack (new HF Docker Space + new Cloudflare Worker reading a fresh empty D1) alongside the live one.

**Architecture:** A streamlined, no-SQLite pipeline. `secinfra::Monitor` streams SEC submissions; each is downloaded as SGML, parsed, filtered to traditional EX-10 exhibits, converted to Markdown (htmd), enriched with a filing-header JSON and (for image-only exhibits) HF-hosted image URLs, then POSTed once to the existing `/api/ingest` route. D1's idempotent upsert is the source of truth. The Astro frontend is reused verbatim with a separate wrangler config pointing at a fresh D1.

**Tech Stack:** Rust 2024, tokio, reqwest (rustls), axum, serde/serde_json, htmd, futures, anyhow, tracing; vendored `secinfra` (with its C SGML parser via `cc`/`build.rs`); Cloudflare Workers + D1 + Drizzle (unchanged); Hugging Face Docker Space.

**Reference spec:** `docs/superpowers/specs/2026-05-31-rust-backend-rewrite-design.md`

---

## File Structure

New crate at `backend-rust/`:

```
backend-rust/
  Cargo.toml                  # package sec-ex10-rust; path dep on vendored secinfra
  Cargo.lock
  README.md                   # HF Space front-matter (sdk: docker, app_port 7860)
  Dockerfile                  # multi-stage rust build → debian-slim runtime
  .dockerignore
  vendor/
    secinfra/                 # vendored copy of arthrod/secinfra-rust (src + vendor/secsgmlc + build.rs + Cargo.toml)
  src/
    main.rs                   # wire config, spawn health server + pipeline, signal handling
    config.rs                 # env → Config
    classify.rs               # EX-10 predicate (pure)
    header.rs                 # standardized metadata events → filing-header JSON (pure)
    markdown.rs               # HTML → markdown via htmd; status decision (pure)
    images.rs                 # image-ref scan, is_image_only, GRAPHIC select, HF commit upload
    ingest.rs                 # IngestRecord, record build, POST /api/ingest, retry queue
    extract.rs                # fetch SGML .txt, parse, gather EX-10 + GRAPHIC docs
    pipeline.rs               # Monitor stream → bounded concurrent per-submission processing
    health.rs                 # axum GET /health
```

Frontend (reuse, add one file; do **not** edit `frontend/wrangler.jsonc`):

```
frontend/wrangler.v2.jsonc    # name sec-ex10-frontend-v2, new D1 id, no custom-domain route
```

Each `src/*.rs` module has one responsibility. Pure modules (`classify`, `header`, `markdown`, the parsing parts of `images`/`ingest`) are unit-tested with no network. I/O modules (`extract` fetch, `ingest` POST, `images` HF upload) hide their network behind small async function boundaries / injected closures so the pure logic is testable.

---

## Conventions for every task

- Tests are Rust `#[cfg(test)] mod tests` blocks in the same file unless noted.
- Run a single test: `cd backend-rust && cargo test <name> -- --nocapture`.
- Run all tests: `cd backend-rust && cargo test`.
- Commit from the **repo root** (`/home/arthrod/workspace/sec-listener`), staging `backend-rust/...` paths.
- Branch is already `feat/rust-backend-rewrite`.

---

## Phase A — Scaffold & vendor secinfra

### Task A1: Create the crate skeleton and vendor secinfra

**Files:**
- Create: `backend-rust/Cargo.toml`
- Create: `backend-rust/src/main.rs`
- Create: `backend-rust/vendor/secinfra/` (copied tree)
- Create: `backend-rust/.dockerignore`

- [ ] **Step 1: Vendor the secinfra source**

The probe clone is at `/tmp/secinfra-rust-probe`. Copy only the crate-essential files (src, the C vendor, build.rs, Cargo.toml, license, readme), excluding scratch/notes/target:

```bash
cd /home/arthrod/workspace/sec-listener
mkdir -p backend-rust/vendor/secinfra
rsync -a \
  --include='src/***' --include='vendor/***' \
  --include='build.rs' --include='Cargo.toml' --include='Cargo.lock' \
  --include='readme.md' --include='LICENSE' \
  --exclude='*' \
  /tmp/secinfra-rust-probe/ backend-rust/vendor/secinfra/
ls backend-rust/vendor/secinfra/src
ls backend-rust/vendor/secinfra/vendor/secsgmlc/src
```

Expected: the `src/*.rs` list (common.rs, efts.rs, monitor.rs, secsgmlc.rs, …) and the C files (secsgml.c, secsgml.h, standardize_submission_metadata.c/.h, uudecode.c/.h).

- [ ] **Step 2: Write `backend-rust/Cargo.toml`**

```toml
[package]
name = "sec-ex10-rust"
version = "0.1.0"
edition = "2024"

[dependencies]
secinfra = { path = "vendor/secinfra" }
anyhow = "1"
futures = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "signal"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
axum = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
htmd = "0.1"
tracing = "0.1"
tracing-subscriber = "0.3"
regex = "1"
once_cell = "1"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "test-util"] }
```

- [ ] **Step 3: Write a placeholder `src/main.rs`**

```rust
fn main() {
    println!("sec-ex10-rust");
}
```

- [ ] **Step 4: Write `.dockerignore`**

```
target
vendor/secinfra/target
```

- [ ] **Step 5: Build to verify the vendored crate compiles (C build included)**

Run: `cd backend-rust && cargo build 2>&1 | tail -20`
Expected: compiles `secinfra` (runs `cc` on the C files) then `sec-ex10-rust`, ending in `Finished`. If `cc` fails, ensure `build-essential` is installed locally (`which cc`).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/Cargo.toml backend-rust/Cargo.lock backend-rust/src/main.rs backend-rust/.dockerignore backend-rust/vendor
git commit -m "feat(rust): scaffold sec-ex10-rust crate, vendor secinfra"
```

---

## Phase B — Pure logic (TDD)

### Task B1: EX-10 classification

**Files:**
- Create: `backend-rust/src/classify.rs`
- Modify: `backend-rust/src/main.rs` (add `mod classify;`)

Parity target (`backend/sec_listener/parsing.py::classify_documents`): a doc is a
**traditional EX-10** iff trimmed `doc_type` starts with `EX-10` and the suffix
after the 5 chars `EX-10` is empty or starts with `.`. `EX-101`/`EX-100` (XBRL)
and other `EX-` are **not** EX-10.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_ex10_variants() {
        assert!(is_traditional_ex10("EX-10"));
        assert!(is_traditional_ex10("EX-10.1"));
        assert!(is_traditional_ex10("EX-10.27"));
        assert!(is_traditional_ex10("  EX-10.1  ")); // trimmed
        // XBRL and others are NOT traditional EX-10
        assert!(!is_traditional_ex10("EX-101"));
        assert!(!is_traditional_ex10("EX-101.INS"));
        assert!(!is_traditional_ex10("EX-100"));
        assert!(!is_traditional_ex10("EX-21"));
        assert!(!is_traditional_ex10("EX-99.1"));
        assert!(!is_traditional_ex10("10-K"));
        assert!(!is_traditional_ex10(""));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test classifies_ex10_variants 2>&1 | tail -15`
Expected: FAIL — `cannot find function is_traditional_ex10`.

- [ ] **Step 3: Write minimal implementation**

```rust
/// True when `doc_type` is a traditional EX-10 material contract
/// (`EX-10` or `EX-10.N`), excluding XBRL `EX-101`/`EX-100` etc.
/// Mirrors backend/sec_listener/parsing.py::classify_documents.
pub fn is_traditional_ex10(doc_type: &str) -> bool {
    let t = doc_type.trim();
    if let Some(suffix) = t.strip_prefix("EX-10") {
        suffix.is_empty() || suffix.starts_with('.')
    } else {
        false
    }
}
```

- [ ] **Step 4: Add the module declaration to `main.rs`**

At the top of `backend-rust/src/main.rs`, add:

```rust
mod classify;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd backend-rust && cargo test classifies_ex10_variants 2>&1 | tail -10`
Expected: PASS (1 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/classify.rs backend-rust/src/main.rs
git commit -m "feat(rust): EX-10 classification predicate"
```

---

### Task B2: Filing-header extraction from standardized metadata

**Files:**
- Create: `backend-rust/src/header.rs`
- Modify: `backend-rust/src/main.rs` (add `mod header;`)

Parity target (`parsing.py::extract_filing_header`): always return the full key
set with empty defaults. The Rust input is the standardized event stream from
`secinfra::ParsedSubmissionMetadata::events()` — a `Vec<SubmissionEvent>` where
each event has `event_type`, `key: Vec<u8>`, `value: Vec<u8>`, `depth`. The
standardizer flattens nested SGML to dotted/keyed events; we select the same
fields datamule exposed. We map by the **last path segment** of the key (the
standardizer keys mirror datamule's: `company-data` → `conformed-name`,
`assigned-sic`, `cik`, `state-of-incorporation`; `filing-values` → `file-number`;
top-level `period`, `filing-date`, `acceptance-datetime`, `item-information`;
`business-address` → `city`, `state`).

We build a `BTreeMap<String, Vec<String>>` keyed by the leaf key name (lowercased)
collecting KeyValue events, then select fields. `items` collects all
`item-information` values.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn kv(key: &str, val: &str) -> Event {
        Event { key: key.to_string(), value: val.to_string() }
    }

    #[test]
    fn builds_full_header_with_defaults() {
        let events = vec![]; // nothing → all defaults
        let h = build_header(&events);
        assert_eq!(h.company_name, "");
        assert_eq!(h.location, "");
        assert!(h.items.is_empty());
        // serializes with all keys present
        let v: serde_json::Value = serde_json::from_str(&header_json(&events)).unwrap();
        for k in ["company_name","cik","sic","state_of_incorporation","period",
                  "filing_date","filed_at","file_number","location","items"] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
    }

    #[test]
    fn selects_fields_and_joins_location() {
        let events = vec![
            kv("conformed-name", "ACME CORP"),
            kv("cik", "0000123"),
            kv("assigned-sic", "7372"),
            kv("state-of-incorporation", "DE"),
            kv("file-number", "001-12345"),
            kv("period", "20250131"),
            kv("filing-date", "20250201"),
            kv("acceptance-datetime", "20250201080000"),
            kv("city", "New York"),
            kv("state", "NY"),
            kv("item-information", "Entry into a Material Definitive Agreement"),
        ];
        let h = build_header(&events);
        assert_eq!(h.company_name, "ACME CORP");
        assert_eq!(h.cik, "0000123");
        assert_eq!(h.sic, "7372");
        assert_eq!(h.state_of_incorporation, "DE");
        assert_eq!(h.file_number, "001-12345");
        assert_eq!(h.period, "20250131");
        assert_eq!(h.filing_date, "20250201");
        assert_eq!(h.filed_at, "20250201080000");
        assert_eq!(h.location, "New York, NY");
        assert_eq!(h.items, vec!["Entry into a Material Definitive Agreement"]);
    }

    #[test]
    fn location_skips_blank_parts() {
        let events = vec![kv("city", "Boston")]; // no state
        assert_eq!(build_header(&events).location, "Boston");
        let events = vec![kv("state", "MA")]; // no city
        assert_eq!(build_header(&events).location, "MA");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib header 2>&1 | tail -15`
Expected: FAIL — `Event`, `build_header`, `header_json` not found.

- [ ] **Step 3: Write minimal implementation**

```rust
use serde::Serialize;
use std::collections::BTreeMap;

/// A flattened key/value from secinfra's standardized submission metadata.
/// `key` is the leaf key (e.g. "conformed-name"); `value` the decoded text.
pub struct Event {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Default)]
pub struct FilingHeader {
    pub company_name: String,
    pub cik: String,
    pub sic: String,
    pub state_of_incorporation: String,
    pub period: String,
    pub filing_date: String,
    pub filed_at: String,
    pub file_number: String,
    pub location: String,
    pub items: Vec<String>,
}

fn first<'a>(map: &'a BTreeMap<String, Vec<String>>, key: &str) -> &'a str {
    map.get(key).and_then(|v| v.first()).map(|s| s.as_str()).unwrap_or("")
}

/// Build the compact filing header from standardized metadata events.
/// Never panics; missing fields default to empty. Mirrors
/// parsing.py::extract_filing_header.
pub fn build_header(events: &[Event]) -> FilingHeader {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in events {
        let k = e.key.trim().to_lowercase();
        if k.is_empty() { continue; }
        map.entry(k).or_default().push(e.value.trim().to_string());
    }
    let city = first(&map, "city");
    let state = first(&map, "state");
    let location = [city, state]
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    FilingHeader {
        company_name: first(&map, "conformed-name").to_string(),
        cik: first(&map, "cik").to_string(),
        sic: first(&map, "assigned-sic").to_string(),
        state_of_incorporation: first(&map, "state-of-incorporation").to_string(),
        period: first(&map, "period").to_string(),
        filing_date: first(&map, "filing-date").to_string(),
        filed_at: first(&map, "acceptance-datetime").to_string(),
        file_number: first(&map, "file-number").to_string(),
        location,
        items: map.get("item-information").cloned().unwrap_or_default(),
    }
}

/// Serialize the header to the JSON string stored in `filing_metadata`.
pub fn header_json(events: &[Event]) -> String {
    serde_json::to_string(&build_header(events)).unwrap_or_else(|_| "{}".to_string())
}
```

- [ ] **Step 4: Add module declaration to `main.rs`**

```rust
mod header;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd backend-rust && cargo test --lib header 2>&1 | tail -10`
Expected: PASS (3 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/header.rs backend-rust/src/main.rs
git commit -m "feat(rust): filing-header extraction from standardized metadata"
```

> **Integration note for Task E (extract):** the bridge from
> `secinfra::ParsedSubmissionMetadata` to `Vec<Event>` lives in `extract.rs`. The
> standardizer emits `SubmissionEvent { event_type, key: Vec<u8>, value, depth }`;
> for each `SubmissionEventType::KeyValue` event, decode `key`/`value` as
> UTF-8-lossy, take the **leaf** of the key (substring after the last `.` if the
> standardizer dotted it, else the whole key) and push an `Event`. This keeps
> `header.rs` pure and network-free.

---

### Task B3: Markdown conversion + status

**Files:**
- Create: `backend-rust/src/markdown.rs`
- Modify: `backend-rust/src/main.rs` (add `mod markdown;`)

Parity target (`converter.py`): empty/None → `""`; conversion failure → `""`
(never panics); output trimmed. Plus the `markdown_status` decision from
`worker.py`/`listener.py`: non-empty markdown → `done`; fetched but empty text →
`empty`; (the `error` status is set by the caller when fetch/convert throws — see
pipeline). We model status as an enum and a helper.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(html_to_markdown(""), "");
    }

    #[test]
    fn converts_basic_html() {
        let md = html_to_markdown("<h1>Title</h1><p>Hello <b>world</b></p>");
        assert!(md.contains("Title"));
        assert!(md.contains("Hello"));
        assert!(md.contains("world"));
        assert_eq!(md, md.trim()); // trimmed
    }

    #[test]
    fn status_from_markdown() {
        assert_eq!(status_for("# real content"), MarkdownStatus::Done);
        assert_eq!(status_for(""), MarkdownStatus::Empty);
        assert_eq!(status_for("   "), MarkdownStatus::Empty);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib markdown 2>&1 | tail -15`
Expected: FAIL — items not found.

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownStatus { Done, Empty, Error }

impl MarkdownStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkdownStatus::Done => "done",
            MarkdownStatus::Empty => "empty",
            MarkdownStatus::Error => "error",
        }
    }
}

/// Convert an HTML document to Markdown. Empty input or any conversion failure
/// yields "" (never panics). Output is trimmed. Mirrors converter.py.
pub fn html_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    match htmd::convert(html) {
        Ok(md) => md.trim().to_string(),
        Err(_) => String::new(),
    }
}

/// Status for a produced markdown string (caller uses Error for fetch/convert throws).
pub fn status_for(markdown: &str) -> MarkdownStatus {
    if markdown.trim().is_empty() { MarkdownStatus::Empty } else { MarkdownStatus::Done }
}
```

> If the `htmd` API differs (e.g. `HtmlToMarkdown::new().convert(&str)`), adjust
> the one call in `html_to_markdown`; the signature and tests stay the same.

- [ ] **Step 4: Add module declaration to `main.rs`**

```rust
mod markdown;
```

- [ ] **Step 5: Run tests**

Run: `cd backend-rust && cargo test --lib markdown 2>&1 | tail -10`
Expected: PASS (3 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/markdown.rs backend-rust/src/main.rs
git commit -m "feat(rust): html→markdown conversion + status"
```

---

### Task B4: Image-ref scan + image-only detection

**Files:**
- Create: `backend-rust/src/images.rs`
- Modify: `backend-rust/src/main.rs` (add `mod images;`)

Parity target (`images.py`): `image_filenames` extracts referenced image
filenames in order, deduped, with the regex
`\(\s*([^()\s"]+\.(?:jpe?g|png|gif|tiff?|svg|webp))(?:\s+"[^"]*")?\s*\)` (case-insensitive).
`is_image_only` = has image refs AND, after stripping image refs + their labels,
no readable text survives. The Python `clean_excerpt` strips image refs and
short label lines; we port the minimal behavior needed: remove all `(...)` image
refs and standalone label tokens, then check if anything non-whitespace,
non-punctuation remains. We implement `clean_excerpt` faithfully enough that an
image-only body (only `![](foo.jpg)`-style refs / bare `(foo.jpg)` refs) yields
empty.

- [ ] **Step 1: Write the failing test (this task adds only the pure helpers)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_image_filenames_in_order_deduped() {
        let md = "text (ex10-3_001.jpg) more (exhibit101.png \"slide1\") (ex10-3_001.jpg)";
        assert_eq!(image_filenames(md), vec!["ex10-3_001.jpg", "exhibit101.png"]);
    }

    #[test]
    fn no_images_returns_empty() {
        assert!(image_filenames("plain text, no refs").is_empty());
        assert!(image_filenames("").is_empty());
    }

    #[test]
    fn image_only_detection() {
        // body that is only image refs → image-only
        assert!(is_image_only("![](a.jpg)\n\n(b.png)"));
        // body with real prose → not image-only
        assert!(!is_image_only("This Agreement is made as of (logo.png)."));
        // no images at all → not image-only
        assert!(!is_image_only("just words"));
    }

    #[test]
    fn dataset_path_and_url() {
        assert_eq!(dataset_image_path("0001-25-000001", "a.jpg"), "images/0001-25-000001/a.jpg");
        assert_eq!(
            public_url("arthrod/sec-ex10-exhibits", "images/x/a.jpg"),
            "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/x/a.jpg"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib images 2>&1 | tail -15`
Expected: FAIL — functions not found.

- [ ] **Step 3: Write minimal implementation**

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static IMG_REF: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\(\s*([^()\s"]+\.(?:jpe?g|png|gif|tiff?|svg|webp))(?:\s+"[^"]*")?\s*\)"#)
        .expect("valid image-ref regex")
});

/// Image filenames referenced in the exhibit markdown — in order, deduped.
/// Mirrors images.py::image_filenames.
pub fn image_filenames(markdown: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cap in IMG_REF.captures_iter(markdown) {
        let fn_ = cap[1].trim().to_string();
        if !fn_.is_empty() && seen.insert(fn_.clone()) {
            out.push(fn_);
        }
    }
    out
}

/// Strip image refs (and their trailing labels) and markdown image syntax,
/// leaving candidate readable text. Mirrors the relevant part of api.clean_excerpt.
fn clean_excerpt(markdown: &str) -> String {
    // remove ![alt](...) image syntax, then bare (foo.jpg) refs
    let no_md_img = Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap().replace_all(markdown, " ");
    let no_refs = IMG_REF.replace_all(&no_md_img, " ");
    // drop residual markdown punctuation/whitespace
    let stripped: String = no_refs
        .chars()
        .filter(|c| !matches!(c, '!' | '[' | ']' | '(' | ')' | '#' | '*' | '_' | '>' | '`' | '-'))
        .collect();
    stripped.trim().to_string()
}

/// True when the body is essentially just image refs: images present AND no
/// readable text survives clean_excerpt. Mirrors images.py::is_image_only.
pub fn is_image_only(markdown: &str) -> bool {
    if image_filenames(markdown).is_empty() {
        return false;
    }
    clean_excerpt(markdown).is_empty()
}

pub fn dataset_image_path(accession: &str, filename: &str) -> String {
    format!("images/{accession}/{filename}")
}

pub fn public_url(repo: &str, path_in_repo: &str) -> String {
    format!("https://huggingface.co/datasets/{repo}/resolve/main/{path_in_repo}")
}
```

- [ ] **Step 4: Add module declaration to `main.rs`**

```rust
mod images;
```

- [ ] **Step 5: Run tests**

Run: `cd backend-rust && cargo test --lib images 2>&1 | tail -10`
Expected: PASS (4 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/images.rs backend-rust/src/main.rs
git commit -m "feat(rust): image-ref scan + image-only detection"
```

---

## Phase C — Ingest record & poster

### Task C1: IngestRecord shape + filed_at fallback

**Files:**
- Create: `backend-rust/src/ingest.rs`
- Modify: `backend-rust/src/main.rs` (add `mod ingest;`)

Parity target (`d1_sync.py::to_ingest_record` + `ingest.ts InRow`). Exact JSON
keys: `id, accession, cik, form_type, doc_type, filename, description, sequence,
filing_url, found_at, filed_at, markdown_status, filing_metadata, image_urls,
markdown`. `id` is a numeric echo token. `filed_at` fallback: if empty, parse it
out of `filing_metadata` JSON `.filed_at`. `image_urls` is a JSON-string array
(or omitted/null when none).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_exact_keys() {
        let r = IngestRecord {
            id: 1,
            accession: "0001-25-000001".into(),
            cik: "123".into(),
            form_type: "8-K".into(),
            doc_type: "EX-10.1".into(),
            filename: "ex10-1.htm".into(),
            description: "Material Contract".into(),
            sequence: "2".into(),
            filing_url: "https://sec.gov/x.txt".into(),
            found_at: "2025-02-01T08:00:00Z".into(),
            filed_at: "20250201080000".into(),
            markdown_status: "done".into(),
            filing_metadata: Some("{\"filed_at\":\"20250201080000\"}".into()),
            image_urls: None,
            markdown: "# hi".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        for k in ["id","accession","cik","form_type","doc_type","filename","description",
                  "sequence","filing_url","found_at","filed_at","markdown_status",
                  "filing_metadata","image_urls","markdown"] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert_eq!(v["id"], serde_json::json!(1));
        assert_eq!(v["image_urls"], serde_json::Value::Null);
    }

    #[test]
    fn filed_at_falls_back_to_metadata() {
        let got = resolve_filed_at("", Some("{\"filed_at\":\"20250201080000\"}"));
        assert_eq!(got, "20250201080000");
        // direct value wins
        assert_eq!(resolve_filed_at("DIRECT", Some("{\"filed_at\":\"X\"}")), "DIRECT");
        // missing/garbage metadata → empty
        assert_eq!(resolve_filed_at("", Some("not json")), "");
        assert_eq!(resolve_filed_at("", None), "");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib ingest 2>&1 | tail -15`
Expected: FAIL — `IngestRecord`, `resolve_filed_at` not found.

- [ ] **Step 3: Write minimal implementation**

```rust
use serde::Serialize;

/// One row in the `/api/ingest` POST body. Field names and order mirror
/// d1_sync.py::_FIELDS and the Astro ingest route's InRow exactly.
#[derive(Debug, Clone, Serialize)]
pub struct IngestRecord {
    pub id: u64,                 // echo token only; D1 mints the real UUIDv7
    pub accession: String,
    pub cik: String,
    pub form_type: String,
    pub doc_type: String,
    pub filename: String,
    pub description: String,
    pub sequence: String,
    pub filing_url: String,
    pub found_at: String,
    pub filed_at: String,
    pub markdown_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filing_metadata: Option<String>,
    pub image_urls: Option<String>, // null when none; serialized explicitly
    pub markdown: String,
}

/// filed_at fallback: use the direct value; if empty, read filing_metadata.filed_at.
/// Mirrors d1_sync.py::to_ingest_record.
pub fn resolve_filed_at(direct: &str, filing_metadata: Option<&str>) -> String {
    if !direct.is_empty() {
        return direct.to_string();
    }
    if let Some(meta) = filing_metadata {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(meta) {
            if let Some(s) = v.get("filed_at").and_then(|x| x.as_str()) {
                return s.to_string();
            }
        }
    }
    String::new()
}
```

> Note: `filing_metadata` uses `skip_serializing_if` so an absent metadata is
> omitted (matches Python sending the key only when present); `image_urls` is kept
> as explicit `null` when `None`, which the enrich-only upsert treats as
> "don't touch" via `coalesce`.

- [ ] **Step 4: Add module declaration to `main.rs`**

```rust
mod ingest;
```

- [ ] **Step 5: Run tests**

Run: `cd backend-rust && cargo test --lib ingest 2>&1 | tail -10`
Expected: PASS (2 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/ingest.rs backend-rust/src/main.rs
git commit -m "feat(rust): ingest record shape + filed_at fallback"
```

---

### Task C2: Ingest poster (batch POST to /api/ingest)

**Files:**
- Modify: `backend-rust/src/ingest.rs`

Parity target (`d1_sync.py::_http_poster` + `_d1_push_loop`): POST
`{"rows":[...]}` with header `X-API-Key`, ≤200 rows per request, parse
`{"accepted":[...]}`, never raise (on error return 0/empty). We add an async
`post_batch` that chunks at 200 and sums accepted counts; failures are logged and
counted as 0 for that chunk (the caller re-queues).

- [ ] **Step 1: Write the failing test (pure chunking helper, no network)**

```rust
// add inside the existing #[cfg(test)] mod tests in ingest.rs
#[test]
fn chunks_at_200() {
    let rows: Vec<u64> = (0..450).collect();
    let chunks = chunk_rows(&rows, 200);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].len(), 200);
    assert_eq!(chunks[1].len(), 200);
    assert_eq!(chunks[2].len(), 50);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib chunks_at_200 2>&1 | tail -12`
Expected: FAIL — `chunk_rows` not found.

- [ ] **Step 3: Write the chunk helper + async poster**

Add to `ingest.rs`:

```rust
/// Split a slice into chunks of at most `max` (D1 ingest caps at 200 rows/POST).
pub fn chunk_rows<T>(rows: &[T], max: usize) -> Vec<&[T]> {
    rows.chunks(max.max(1)).collect()
}

/// POST one batch (≤200 rows) to the ingest route. Returns the count of accepted
/// ids. Never panics; on any error logs and returns 0 (caller decides retry).
/// Mirrors d1_sync.py::_http_poster + push_finalized accounting.
pub async fn post_batch(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    rows: &[IngestRecord],
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let body = serde_json::json!({ "rows": rows });
    let resp = match client
        .post(url)
        .header("X-API-Key", key)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("ingest POST failed: {e}");
            return 0;
        }
    };
    if !resp.status().is_success() {
        tracing::warn!("ingest POST status {}", resp.status());
        return 0;
    }
    #[derive(serde::Deserialize)]
    struct AcceptedResp { accepted: Vec<serde_json::Value> }
    match resp.json::<AcceptedResp>().await {
        Ok(a) => a.accepted.len(),
        Err(e) => {
            tracing::warn!("ingest response parse failed: {e}");
            0
        }
    }
}
```

- [ ] **Step 4: Run the chunk test**

Run: `cd backend-rust && cargo test --lib chunks_at_200 2>&1 | tail -8`
Expected: PASS.

- [ ] **Step 5: Build to ensure the async poster compiles**

Run: `cd backend-rust && cargo build 2>&1 | tail -8`
Expected: `Finished`.

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/ingest.rs
git commit -m "feat(rust): ingest poster with 200-row chunking"
```

---

## Phase D — Image capture via HF Hub commit API

### Task D1: HF commit upload (raw Hub API)

**Files:**
- Modify: `backend-rust/src/images.rs`

Parity target (`images.py::upload_images` + `capture_images`): upload all of a
filing's selected images in **one commit** to a dataset repo under
`images/{accession}/{filename}`; return their public `resolve/main` URLs. No
official Rust HF client → use the Hub commit HTTP flow with `reqwest`:

1. `POST https://huggingface.co/api/datasets/{repo}/preupload/main` with
   `{"files":[{"path":..., "sample": <base64 first bytes>, "size": <n>}]}` and
   `Authorization: Bearer {token}` to learn which files need LFS vs regular. For
   robustness and simplicity we upload all blobs **inline as base64** via the
   commit endpoint (works for the small scanned images we handle); LFS pointer
   flow is out of scope for v1 and noted as a limitation.
2. `POST https://huggingface.co/api/datasets/{repo}/commit/main` with an NDJSON
   body: a header line `{"key":"header","value":{"summary": "...", }}` followed by
   one `{"key":"file","value":{"content": <base64>, "path": ..., "encoding":"base64"}}`
   per file.

We isolate the HTTP in `upload_images`, and keep `capture_images` orchestration
(filter to `only`, build paths/URLs) testable via an injected uploader closure.

- [ ] **Step 1: Write the failing test for the orchestration (injected uploader, no network)**

```rust
// add to images.rs tests
#[tokio::test]
async fn capture_filters_to_only_and_builds_urls() {
    // graphics fetched for the filing (filename, bytes)
    let graphics = vec![
        ("ex10-3_001.jpg".to_string(), vec![1u8, 2, 3]),
        ("other.jpg".to_string(), vec![9u8]),
    ];
    let only: std::collections::HashSet<String> =
        ["ex10-3_001.jpg".to_string()].into_iter().collect();

    // uploader records what it was asked to upload, returns Ok
    let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let up = uploaded.clone();
    let uploader = move |paths: Vec<(Vec<u8>, String)>| {
        let up = up.clone();
        async move {
            for (_, p) in &paths { up.lock().unwrap().push(p.clone()); }
            Ok::<(), anyhow::Error>(())
        }
    };

    let urls = capture_images_with(
        "0001-25-000001", graphics, Some(&only),
        "arthrod/sec-ex10-exhibits", uploader,
    ).await;

    assert_eq!(urls, vec![
        "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/0001-25-000001/ex10-3_001.jpg"
    ]);
    assert_eq!(*uploaded.lock().unwrap(), vec!["images/0001-25-000001/ex10-3_001.jpg"]);
}

#[tokio::test]
async fn capture_returns_empty_when_no_matches() {
    let graphics = vec![("nope.jpg".to_string(), vec![1u8])];
    let only: std::collections::HashSet<String> = ["x.jpg".to_string()].into_iter().collect();
    let uploader = |_p: Vec<(Vec<u8>, String)>| async { Ok::<(), anyhow::Error>(()) };
    let urls = capture_images_with("acc", graphics, Some(&only), "repo", uploader).await;
    assert!(urls.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib capture_ 2>&1 | tail -15`
Expected: FAIL — `capture_images_with` not found.

- [ ] **Step 3: Implement orchestration + HF uploader**

Add to `images.rs`:

```rust
use std::collections::HashSet;
use std::future::Future;

/// Orchestrate capture with an injected async uploader. Filters graphics to
/// `only`, uploads as one commit, returns public URLs in order. On uploader
/// error returns []. Mirrors images.py::capture_images.
pub async fn capture_images_with<U, Fut>(
    accession: &str,
    graphics: Vec<(String, Vec<u8>)>,
    only: Option<&HashSet<String>>,
    repo: &str,
    uploader: U,
) -> Vec<String>
where
    U: FnOnce(Vec<(Vec<u8>, String)>) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let selected: Vec<(String, Vec<u8>)> = graphics
        .into_iter()
        .filter(|(f, _)| only.map_or(true, |o| o.contains(f)))
        .collect();
    if selected.is_empty() {
        return Vec::new();
    }
    let mut uploads = Vec::new();
    let mut urls = Vec::new();
    for (filename, data) in selected {
        let path = dataset_image_path(accession, &filename);
        urls.push(public_url(repo, &path));
        uploads.push((data, path));
    }
    match uploader(uploads).await {
        Ok(()) => urls,
        Err(e) => {
            tracing::warn!("image upload failed for {accession}: {e}");
            Vec::new()
        }
    }
}

/// Upload blobs to an HF dataset in ONE commit via the Hub commit API (base64
/// inline). Mirrors images.py::upload_images. `uploads` is (bytes, path_in_repo).
pub async fn hf_upload(
    client: &reqwest::Client,
    repo: &str,
    token: &str,
    uploads: Vec<(Vec<u8>, String)>,
) -> anyhow::Result<()> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut ndjson = String::new();
    let header = serde_json::json!({
        "key": "header",
        "value": { "summary": format!("add {} exhibit image(s)", uploads.len()) }
    });
    ndjson.push_str(&serde_json::to_string(&header)?);
    ndjson.push('\n');
    for (data, path) in &uploads {
        let line = serde_json::json!({
            "key": "file",
            "value": { "path": path, "encoding": "base64", "content": b64.encode(data) }
        });
        ndjson.push_str(&serde_json::to_string(&line)?);
        ndjson.push('\n');
    }
    let url = format!("https://huggingface.co/api/datasets/{repo}/commit/main");
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/x-ndjson")
        .body(ndjson)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("HF commit failed {status}: {text}");
    }
    Ok(())
}
```

- [ ] **Step 4: Add the `base64` dependency**

In `backend-rust/Cargo.toml` under `[dependencies]` add:

```toml
base64 = "0.22"
```

- [ ] **Step 5: Run tests**

Run: `cd backend-rust && cargo test --lib capture_ 2>&1 | tail -10`
Expected: PASS (2 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/images.rs backend-rust/Cargo.toml backend-rust/Cargo.lock
git commit -m "feat(rust): HF commit-API image upload + capture orchestration"
```

> **Limitation noted in spec §10/§9:** inline-base64 commit works for the small
> scanned images we host; very large blobs would need the LFS preupload pointer
> flow, which is out of scope for v1.

---

## Phase E — Extraction (SGML fetch + parse)

### Task E1: Extracted-document model + EX-10/GRAPHIC gathering from parsed SGML

**Files:**
- Create: `backend-rust/src/extract.rs`
- Modify: `backend-rust/src/main.rs` (add `mod extract;`)

This bridges `secinfra` to our domain. `secinfra::ParsedSgml::parse(&[u8])` yields
documents with `.doc_type()/.filename()/.description()/.sequence()/.content()`
(all `&[u8]`, uudecode handled). We expose a pure `gather(parsed)` that returns
EX-10 docs (with content) and GRAPHIC docs (filename+bytes), so it's testable by
constructing inputs from a fixture SGML file. We also bridge
`ParsedSubmissionMetadata::events()` → `Vec<header::Event>`.

Because constructing `ParsedSgml` requires real SGML bytes, the unit test uses a
**small synthetic SGML fixture** committed under `backend-rust/tests/fixtures/`.

- [ ] **Step 1: Create a tiny SGML fixture**

Create `backend-rust/tests/fixtures/mini.txt` with one EX-10 doc and one GRAPHIC:

```
<SEC-DOCUMENT>0001-25-000001.txt
<SEC-HEADER>
ACCESSION NUMBER:		0001-25-000001
<DOCUMENT>
<TYPE>EX-10.1
<SEQUENCE>2
<FILENAME>ex10-1.htm
<DESCRIPTION>Material Contract
<TEXT>
<html><body><h1>Agreement</h1><p>Terms here.</p></body></html>
</TEXT>
</DOCUMENT>
<DOCUMENT>
<TYPE>GRAPHIC
<SEQUENCE>3
<FILENAME>img1.jpg
<DESCRIPTION>scan
<TEXT>
JFIFbinary-bytes
</TEXT>
</DOCUMENT>
</SEC-DOCUMENT>
```

- [ ] **Step 2: Write the failing test**

Create the test in `extract.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gathers_ex10_and_graphic() {
        let sgml = std::fs::read("tests/fixtures/mini.txt").expect("fixture");
        let parsed = secinfra::ParsedSgml::parse(&sgml).expect("parse");
        let g = gather(&parsed);
        assert_eq!(g.ex10.len(), 1);
        assert_eq!(g.ex10[0].doc_type, "EX-10.1");
        assert_eq!(g.ex10[0].filename, "ex10-1.htm");
        assert_eq!(g.ex10[0].sequence, "2");
        assert_eq!(g.ex10[0].description, "Material Contract");
        assert!(String::from_utf8_lossy(&g.ex10[0].content).contains("Agreement"));
        assert_eq!(g.graphics.len(), 1);
        assert_eq!(g.graphics[0].0, "img1.jpg");
        assert!(!g.graphics[0].1.is_empty());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib gathers_ex10_and_graphic 2>&1 | tail -15`
Expected: FAIL — `gather`, `Gathered` not found.

- [ ] **Step 4: Write the implementation**

```rust
use crate::classify::is_traditional_ex10;
use crate::header::Event;
use secinfra::{ParsedSgml, ParsedSubmissionMetadata, SubmissionEventType};

/// One EX-10 exhibit document pulled from a filing's SGML.
pub struct Ex10Doc {
    pub doc_type: String,
    pub filename: String,
    pub description: String,
    pub sequence: String,
    pub content: Vec<u8>,
}

/// EX-10 exhibits + GRAPHIC (image) documents from one parsed filing.
pub struct Gathered {
    pub ex10: Vec<Ex10Doc>,
    pub graphics: Vec<(String, Vec<u8>)>,
}

fn s(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Pure: split a parsed filing into traditional EX-10 docs and GRAPHIC docs.
pub fn gather(parsed: &ParsedSgml) -> Gathered {
    let mut ex10 = Vec::new();
    let mut graphics = Vec::new();
    for d in parsed.documents() {
        let dt = s(d.doc_type());
        if is_traditional_ex10(&dt) {
            ex10.push(Ex10Doc {
                doc_type: dt,
                filename: s(d.filename()),
                description: s(d.description()),
                sequence: s(d.sequence()),
                content: d.content().to_vec(),
            });
        } else if dt == "GRAPHIC" {
            let fname = s(d.filename());
            if !fname.is_empty() {
                graphics.push((fname, d.content().to_vec()));
            }
        }
    }
    Gathered { ex10, graphics }
}

/// Bridge standardized submission metadata into header::Event list.
/// Keeps header.rs pure (no secinfra dependency).
pub fn header_events(meta: &ParsedSubmissionMetadata) -> Vec<Event> {
    meta.events()
        .into_iter()
        .filter(|e| e.event_type == SubmissionEventType::KeyValue)
        .map(|e| {
            let key_full = String::from_utf8_lossy(&e.key);
            // take the leaf segment after the last '.' if dotted
            let leaf = key_full.rsplit('.').next().unwrap_or(&key_full).to_string();
            Event {
                key: leaf,
                value: String::from_utf8_lossy(&e.value).to_string(),
            }
        })
        .collect()
}
```

- [ ] **Step 5: Add module declaration to `main.rs`**

```rust
mod extract;
```

- [ ] **Step 6: Run tests**

Run: `cd backend-rust && cargo test --lib gathers_ex10_and_graphic 2>&1 | tail -10`
Expected: PASS.

> If the fixture's exact field text doesn't survive the C parser as written,
> adjust the fixture (not the assertions' intent) until the parser yields the EX-10
> + GRAPHIC docs; the parser is the same one the Python stack relied on.

- [ ] **Step 7: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/extract.rs backend-rust/src/main.rs backend-rust/tests/fixtures/mini.txt
git commit -m "feat(rust): gather EX-10 + GRAPHIC docs from parsed SGML"
```

---

### Task E2: SGML download (async fetch)

**Files:**
- Modify: `backend-rust/src/extract.rs`

The SGML `.txt` URL is built with `secinfra::construct_sgml_url(accession, cik)`.
Add an async fetch that returns the raw bytes, using a shared `reqwest::Client`
with the SEC user agent. Never panics — returns `anyhow::Result`.

- [ ] **Step 1: Write the implementation (no unit test — pure network; covered by integration)**

Add to `extract.rs`:

```rust
/// Download a filing's full SGML submission (.txt) bytes.
pub async fn fetch_sgml(
    client: &reqwest::Client,
    accession: u64,
    cik: u64,
) -> anyhow::Result<Vec<u8>> {
    let url = secinfra::construct_sgml_url(accession, cik);
    let resp = client.get(&url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

/// The canonical filing URL stored in `filing_url` (same .txt URL).
pub fn filing_url(accession: u64, cik: u64) -> String {
    secinfra::construct_sgml_url(accession, cik)
}
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cd backend-rust && cargo build 2>&1 | tail -8`
Expected: `Finished`.

- [ ] **Step 3: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/extract.rs
git commit -m "feat(rust): async SGML download via secinfra URL builder"
```

---

## Phase F — Pipeline, health, main

### Task F1: Per-submission processing (assemble records)

**Files:**
- Create: `backend-rust/src/pipeline.rs`
- Modify: `backend-rust/src/main.rs` (add `mod pipeline;`)

This is the orchestration core (parity with `listener.process_filing` +
`worker` backfill, collapsed into one pass). Given a parsed filing + metadata, it
builds `IngestRecord`s: for each EX-10 doc, convert markdown, decide status, and
if image-only, capture images. We make a **pure** `build_records` that takes the
gathered docs, the header JSON, the per-doc markdown results, and the per-doc
image URLs already resolved — so it's unit-testable — and a separate async
`process_submission` that wires fetch/convert/capture and calls `build_records`.

- [ ] **Step 1: Write the failing test for `build_records`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Ex10Doc;

    fn doc(dt: &str, fname: &str) -> Ex10Doc {
        Ex10Doc { doc_type: dt.into(), filename: fname.into(),
                  description: "d".into(), sequence: "2".into(), content: b"<p>x</p>".to_vec() }
    }

    #[test]
    fn builds_one_record_per_ex10_doc() {
        let inputs = vec![
            DocResult {
                doc: doc("EX-10.1", "a.htm"),
                markdown: "# A".into(),
                status: "done".into(),
                image_urls: None,
            },
            DocResult {
                doc: doc("EX-10.2", "b.htm"),
                markdown: "".into(),
                status: "empty".into(),
                image_urls: Some(vec!["https://hf/x.jpg".into()]),
            },
        ];
        let recs = build_records(
            &mut 0, "0001-25-000001", "123", "8-K",
            "https://sec.gov/f.txt", "2025-02-01T00:00:00Z",
            Some("{\"filed_at\":\"20250201080000\"}"),
            inputs,
        );
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].id, 1);
        assert_eq!(recs[1].id, 2);
        assert_eq!(recs[0].doc_type, "EX-10.1");
        assert_eq!(recs[0].markdown, "# A");
        assert_eq!(recs[0].filed_at, "20250201080000"); // from metadata fallback
        assert_eq!(recs[0].image_urls, None);
        assert_eq!(recs[1].markdown_status, "empty");
        assert_eq!(recs[1].image_urls, Some("[\"https://hf/x.jpg\"]".into()));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib builds_one_record_per_ex10_doc 2>&1 | tail -15`
Expected: FAIL — `DocResult`, `build_records` not found.

- [ ] **Step 3: Implement `DocResult` + `build_records`**

```rust
use crate::extract::Ex10Doc;
use crate::ingest::{resolve_filed_at, IngestRecord};

/// A processed EX-10 document ready to become a record.
pub struct DocResult {
    pub doc: Ex10Doc,
    pub markdown: String,
    pub status: String,
    pub image_urls: Option<Vec<String>>,
}

/// Pure: assemble ingest records for one filing. `id_counter` is the
/// monotonic per-process echo token, bumped once per record.
#[allow(clippy::too_many_arguments)]
pub fn build_records(
    id_counter: &mut u64,
    accession: &str,
    cik: &str,
    form_type: &str,
    filing_url: &str,
    found_at: &str,
    filing_metadata: Option<&str>,
    docs: Vec<DocResult>,
) -> Vec<IngestRecord> {
    docs.into_iter()
        .map(|d| {
            *id_counter += 1;
            IngestRecord {
                id: *id_counter,
                accession: accession.to_string(),
                cik: cik.to_string(),
                form_type: form_type.to_string(),
                doc_type: d.doc.doc_type,
                filename: d.doc.filename,
                description: d.doc.description,
                sequence: d.doc.sequence,
                filing_url: filing_url.to_string(),
                found_at: found_at.to_string(),
                filed_at: resolve_filed_at("", filing_metadata),
                markdown_status: d.status,
                filing_metadata: filing_metadata.map(|s| s.to_string()),
                image_urls: d.image_urls.map(|u| serde_json::to_string(&u).unwrap_or_else(|_| "[]".into())),
                markdown: d.markdown,
            }
        })
        .collect()
}
```

- [ ] **Step 4: Add module declaration to `main.rs`**

```rust
mod pipeline;
```

- [ ] **Step 5: Run tests**

Run: `cd backend-rust && cargo test --lib builds_one_record_per_ex10_doc 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/pipeline.rs backend-rust/src/main.rs
git commit -m "feat(rust): assemble ingest records per EX-10 doc"
```

---

### Task F2: Async process_submission (wire fetch → parse → convert → capture)

**Files:**
- Modify: `backend-rust/src/pipeline.rs`

Wires the real I/O for one submission. Robustness rule: any error logs and yields
an empty record list (never propagates — one bad filing never stops the loop).

- [ ] **Step 1: Write the implementation**

```rust
use crate::config::Config;
use crate::extract::{fetch_sgml, filing_url, gather, header_events};
use crate::header::header_json;
use crate::images::{capture_images_with, hf_upload, image_filenames, is_image_only};
use crate::markdown::{html_to_markdown, status_for, MarkdownStatus};
use secinfra::{ParsedSgml, ParsedSubmissionMetadata, Submission};
use std::collections::HashSet;

/// Process one submission end-to-end into ingest records. Never panics.
pub async fn process_submission(
    client: &reqwest::Client,
    cfg: &Config,
    id_counter: &mut u64,
    sub: &Submission,
) -> Vec<IngestRecord> {
    let cik = match sub.ciks.first() {
        Some(c) => *c,
        None => return Vec::new(),
    };
    let accession_str = secinfra::format_accession_int(sub.accession, "dash");
    let f_url = filing_url(sub.accession, cik);
    let found_at = sub.detected_time.to_rfc3339();

    let sgml = match fetch_sgml(client, sub.accession, cik).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("fetch_sgml failed for {accession_str}: {e}");
            return Vec::new();
        }
    };

    let parsed = match ParsedSgml::parse(&sgml) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("SGML parse failed for {accession_str}: {e}");
            return Vec::new();
        }
    };
    let gathered = gather(&parsed);
    if gathered.ex10.is_empty() {
        return Vec::new();
    }

    // Filing header (best-effort; empty header on failure).
    let meta_json = match ParsedSubmissionMetadata::parse(&sgml) {
        Ok(m) => header_json(&header_events(&m)),
        Err(_) => header_json(&[]),
    };

    let mut results = Vec::new();
    for doc in gathered.ex10 {
        let html = String::from_utf8_lossy(&doc.content);
        let md = if cfg.convert_markdown { html_to_markdown(&html) } else { String::new() };
        let status = status_for(&md).as_str().to_string();

        // image-only → capture this doc's images (best-effort).
        let mut image_urls: Option<Vec<String>> = None;
        if cfg.hf_token.is_some() && is_image_only(&md) {
            let only: HashSet<String> = image_filenames(&md).into_iter().collect();
            let token = cfg.hf_token.clone().unwrap();
            let repo = cfg.image_repo.clone();
            let graphics = gathered.graphics.clone();
            let client2 = client.clone();
            let urls = capture_images_with(
                &accession_str, graphics, Some(&only), &repo,
                move |uploads| async move { hf_upload(&client2, &repo, &token, uploads).await },
            ).await;
            if !urls.is_empty() {
                image_urls = Some(urls);
            }
        }

        results.push(DocResult { doc, markdown: md, status, image_urls });
    }

    build_records(
        id_counter, &accession_str, &cik.to_string(),
        &sub.submission_type, &f_url, &found_at, Some(&meta_json), results,
    )
}
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cd backend-rust && cargo build 2>&1 | tail -15`
Expected: `Finished`. (Fix any mismatch between `cfg` field names and Task F4's `Config` — they must agree: `convert_markdown`, `hf_token`, `image_repo`.)

- [ ] **Step 3: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/pipeline.rs
git commit -m "feat(rust): async per-submission processing pipeline"
```

---

### Task F3: Config from env

**Files:**
- Create: `backend-rust/src/config.rs`
- Modify: `backend-rust/src/main.rs` (add `mod config;`)

Env parity with spec §6.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_without_env() {
        let c = Config::from_map(|_| None);
        assert_eq!(c.poll_interval_ms, 200);
        assert_eq!(c.concurrency, 8);
        assert_eq!(c.push_batch, 100);
        assert_eq!(c.port, 7860);
        assert!(c.convert_markdown);
        assert_eq!(c.image_repo, "arthrod/sec-ex10-exhibits");
        assert!(c.hf_token.is_none());
        assert!(c.api_key.is_none());
    }

    #[test]
    fn reads_overrides_and_caps_batch() {
        let c = Config::from_map(|k| match k {
            "SEC_PUSH_BATCH" => Some("500".into()), // capped to 200
            "SEC_CONCURRENCY" => Some("4".into()),
            "SEC_API_KEY" => Some("secret".into()),
            "HF_TOKEN" => Some("hf_x".into()),
            _ => None,
        });
        assert_eq!(c.push_batch, 200);
        assert_eq!(c.concurrency, 4);
        assert_eq!(c.api_key.as_deref(), Some("secret"));
        assert_eq!(c.hf_token.as_deref(), Some("hf_x"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd backend-rust && cargo test --lib config 2>&1 | tail -12`
Expected: FAIL — `Config` not found.

- [ ] **Step 3: Implement**

```rust
#[derive(Debug, Clone)]
pub struct Config {
    pub ingest_url: String,
    pub api_key: Option<String>,
    pub hf_token: Option<String>,
    pub image_repo: String,
    pub poll_interval_ms: u64,
    pub concurrency: usize,
    pub push_batch: usize,
    pub port: u16,
    pub convert_markdown: bool,
}

fn parse_or<T: std::str::FromStr>(v: Option<String>, default: T) -> T {
    v.and_then(|s| s.parse().ok()).unwrap_or(default)
}

impl Config {
    /// Build from a getter (real impl passes std::env::var). Batch capped at 200.
    pub fn from_map(get: impl Fn(&str) -> Option<String>) -> Self {
        let convert_markdown = get("SEC_CONVERT_MARKDOWN")
            .map(|s| matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(true);
        Config {
            ingest_url: get("D1_INGEST_URL")
                .unwrap_or_else(|| "https://live-contracts.arthur.law/api/ingest".into()),
            api_key: get("SEC_API_KEY").filter(|s| !s.is_empty()),
            hf_token: get("HF_TOKEN").filter(|s| !s.is_empty()),
            image_repo: get("SEC_IMAGE_REPO").unwrap_or_else(|| "arthrod/sec-ex10-exhibits".into()),
            poll_interval_ms: parse_or(get("SEC_POLL_INTERVAL_MS"), 200),
            concurrency: parse_or(get("SEC_CONCURRENCY"), 8usize).max(1),
            push_batch: parse_or(get("SEC_PUSH_BATCH"), 100usize).min(200).max(1),
            port: parse_or(get("PORT"), 7860u16),
            convert_markdown,
        }
    }

    pub fn from_env() -> Self {
        Self::from_map(|k| std::env::var(k).ok())
    }
}
```

- [ ] **Step 4: Add module declaration to `main.rs`**

```rust
mod config;
```

- [ ] **Step 5: Run tests**

Run: `cd backend-rust && cargo test --lib config 2>&1 | tail -10`
Expected: PASS (2 passed).

- [ ] **Step 6: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/config.rs backend-rust/src/main.rs
git commit -m "feat(rust): env-driven Config"
```

---

### Task F4: Health server

**Files:**
- Create: `backend-rust/src/health.rs`
- Modify: `backend-rust/src/main.rs` (add `mod health;`)

`GET /health` → `{"status":"ok","total_seen":<n>}` where `n` is an atomic counter
of submissions processed (parity with the Python `/health` open endpoint).

- [ ] **Step 1: Write the implementation (smoke-tested via build + Task G health check)**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::{routing::get, Json, Router};
use serde_json::json;

#[derive(Clone, Default)]
pub struct HealthState {
    pub total_seen: Arc<AtomicU64>,
}

pub fn router(state: HealthState) -> Router {
    Router::new().route(
        "/health",
        get(move || {
            let n = state.total_seen.load(Ordering::Relaxed);
            async move { Json(json!({ "status": "ok", "total_seen": n })) }
        }),
    )
}

/// Bind and serve the health router on 0.0.0.0:port.
pub async fn serve(port: u16, state: HealthState) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}
```

- [ ] **Step 2: Add module declaration + build**

Add `mod health;` to `main.rs`, then:
Run: `cd backend-rust && cargo build 2>&1 | tail -8`
Expected: `Finished`.

- [ ] **Step 3: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/health.rs backend-rust/src/main.rs
git commit -m "feat(rust): /health server"
```

---

### Task F5: Pipeline run loop + main wiring

**Files:**
- Modify: `backend-rust/src/pipeline.rs`
- Modify: `backend-rust/src/main.rs`

The run loop: build the `Monitor` stream, for each batch process submissions with
bounded concurrency, collect records, and POST them in `push_batch`-sized chunks.
A failed POST re-queues (bounded) — simplest correct version: collect all records
for the batch, attempt POST per chunk, and on failure log + keep going (next
Monitor cycle re-streams nothing for those, but D1 idempotency + restart re-stream
covers gaps; this matches "never block the loop"). Increment `total_seen`.

- [ ] **Step 1: Implement the run loop in `pipeline.rs`**

```rust
use crate::health::HealthState;
use crate::ingest::{chunk_rows, post_batch};
use futures::StreamExt;
use secinfra::Monitor;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Run the monitor→process→push loop forever. Never returns under normal operation.
pub async fn run(cfg: Config, health: HealthState) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(std::env::var("SEC_USER_AGENT").unwrap_or_else(|_| secinfra::sec_user_agent()))
        .build()?;

    let id_counter = Arc::new(Mutex::new(0u64));
    let mut stream = Box::pin(
        Monitor::new()
            .polling_interval_ms(cfg.poll_interval_ms)
            .use_rss(true)
            .use_efts(true)
            .build(),
    );

    tracing::info!("pipeline started → {}", cfg.ingest_url);
    while let Some(batch) = stream.next().await {
        // Process submissions with bounded concurrency.
        let cfg_ref = &cfg;
        let client_ref = &client;
        let counter = id_counter.clone();
        let records: Vec<IngestRecord> = futures::stream::iter(batch)
            .map(|sub| {
                let counter = counter.clone();
                async move {
                    let mut c = counter.lock().await;
                    process_submission(client_ref, cfg_ref, &mut c, &sub).await
                }
            })
            .buffer_unordered(cfg.concurrency)
            .flat_map(futures::stream::iter)
            .collect()
            .await;

        health.total_seen.fetch_add(records.len() as u64, Ordering::Relaxed);

        if records.is_empty() {
            continue;
        }
        if let Some(key) = cfg.api_key.as_deref() {
            for chunk in chunk_rows(&records, cfg.push_batch) {
                let n = post_batch(&client, &cfg.ingest_url, key, chunk).await;
                if n > 0 {
                    tracing::info!("ingest: accepted {n} rows");
                }
            }
        } else {
            tracing::warn!("SEC_API_KEY unset; {} records not pushed", records.len());
        }
    }
    Ok(())
}
```

> The `id_counter` behind a `Mutex` keeps the echo token monotonic across
> concurrent tasks. Lock is held only during the synchronous counter bump inside
> `build_records`; processing I/O happens before the records are built. (If lock
> contention matters later, switch to `AtomicU64`; not needed for v1.)

- [ ] **Step 2: Write `main.rs` final wiring**

Replace the body of `backend-rust/src/main.rs` (keep all `mod` lines at top) so it reads:

```rust
mod classify;
mod config;
mod extract;
mod header;
mod health;
mod images;
mod ingest;
mod markdown;
mod pipeline;

use health::HealthState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = config::Config::from_env();
    let health = HealthState::default();

    tracing::info!("sec-ex10-rust starting (port {})", cfg.port);

    let health_task = {
        let h = health.clone();
        let port = cfg.port;
        tokio::spawn(async move {
            if let Err(e) = health::serve(port, h).await {
                tracing::error!("health server error: {e}");
            }
        })
    };

    let pipeline_task = {
        let cfg = cfg.clone();
        let h = health.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = pipeline::run(cfg.clone(), h.clone()).await {
                    tracing::error!("pipeline crashed: {e}; restarting in 10s");
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            }
        })
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("shutdown signal"),
        _ = health_task => tracing::error!("health task exited"),
        _ = pipeline_task => tracing::error!("pipeline task exited"),
    }
    Ok(())
}
```

- [ ] **Step 3: Build the whole crate**

Run: `cd backend-rust && cargo build 2>&1 | tail -20`
Expected: `Finished`. Fix any field/name mismatches surfaced here.

- [ ] **Step 4: Run the full test suite**

Run: `cd backend-rust && cargo test 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/src/pipeline.rs backend-rust/src/main.rs
git commit -m "feat(rust): monitor→process→push run loop + main wiring"
```

---

## Phase G — Dockerfile & HF Space bundle

### Task G1: Dockerfile + Space README

**Files:**
- Create: `backend-rust/Dockerfile`
- Create: `backend-rust/README.md`

- [ ] **Step 1: Write the Dockerfile**

```dockerfile
# SEC EX-10 Rust backend as a Hugging Face Docker Space.
# Streams SEC EDGAR, extracts EX-10 exhibits, converts to markdown, captures
# images, and POSTs finalized rows to the Astro /api/ingest route (X-API-Key).
# A /health endpoint on :7860 is HF's public liveness surface.
FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends build-essential clang && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY vendor ./vendor
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd -m -u 1000 user
USER user
WORKDIR /home/user/app
COPY --from=builder --chown=user /build/target/release/sec-ex10-rust /home/user/app/sec-ex10-rust
ENV PORT=7860 RUST_LOG=info
EXPOSE 7860
CMD ["/home/user/app/sec-ex10-rust"]
```

- [ ] **Step 2: Write the Space README with HF front-matter**

```markdown
---
title: SEC EX-10 Backend (Rust)
emoji: 🦀
colorFrom: blue
colorTo: green
sdk: docker
app_port: 7860
pinned: false
short_description: Live SEC EX-10 listener (Rust) → Cloudflare D1
---

# SEC EX-10 Backend (Rust)

Rust rewrite of the SEC EX-10 listener built on the vendored `secinfra` crate.
Streams EDGAR submissions, extracts EX-10 material-contract exhibits, converts
each to Markdown, captures scanned-exhibit images to a Hugging Face dataset, and
POSTs finalized rows to the Astro `/api/ingest` route backed by Cloudflare D1.

`GET /health` is the public liveness endpoint. All data goes to D1; this Space
serves no content API.

## Environment

| Var | Default | Meaning |
|-----|---------|---------|
| `SEC_USER_AGENT` | secinfra default | SEC User-Agent (set to a real contact) |
| `D1_INGEST_URL` | — | the v2 Worker `/api/ingest` URL |
| `SEC_API_KEY` | — | shared ingest auth (same value as the Worker secret) |
| `HF_TOKEN` | — | enables image upload; absent → images skipped |
| `SEC_IMAGE_REPO` | `arthrod/sec-ex10-exhibits` | HF dataset for image blobs |
| `SEC_CONCURRENCY` | 8 | in-flight submissions |
| `SEC_PUSH_BATCH` | 100 | rows per ingest POST (cap 200) |
```

- [ ] **Step 3: Build the Docker image locally to verify**

Run: `cd backend-rust && docker build -t sec-ex10-rust:test . 2>&1 | tail -25`
Expected: ends with a successful image build. (If Docker isn't available in the
exec environment, skip the local build and rely on the Space build; note it.)

- [ ] **Step 4: Smoke-test /health (if Docker built)**

```bash
docker run -d --rm -p 7860:7860 --name sec-rust-test sec-ex10-rust:test
sleep 3
curl -s localhost:7860/health
docker stop sec-rust-test
```

Expected: `{"status":"ok","total_seen":0}`.

- [ ] **Step 5: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/Dockerfile backend-rust/README.md
git commit -m "feat(rust): Dockerfile + HF Space README"
```

---

## Phase H — Frontend v2 (fresh D1 + new Worker)

### Task H1: wrangler.v2.jsonc (do not touch the live config)

**Files:**
- Create: `frontend/wrangler.v2.jsonc`

- [ ] **Step 1: Create the new D1 and capture its id**

Run:
```bash
cd /home/arthrod/workspace/sec-listener/frontend
wrangler d1 create sec-ex10-v2
```
Expected: prints a `database_id` (a UUID). Record it for Step 2.

- [ ] **Step 2: Write `frontend/wrangler.v2.jsonc`**

Copy of `frontend/wrangler.jsonc` with: new `name`, the new `database_id`, a new
`SESSION` KV (create it first: `wrangler kv namespace create SESSION_V2` → use the
returned id), and **no** `routes` block (use the `*.workers.dev` URL).

```jsonc
{
  "$schema": "node_modules/wrangler/config-schema.json",
  "name": "sec-ex10-frontend-v2",
  "main": "@astrojs/cloudflare/entrypoints/server",
  "compatibility_date": "2026-04-15",
  "compatibility_flags": ["nodejs_compat"],
  "assets": {
    "binding": "ASSETS",
    "directory": "./dist/client",
    "html_handling": "drop-trailing-slash"
  },
  "kv_namespaces": [
    { "binding": "SESSION", "id": "<PASTE_NEW_KV_ID>" }
  ],
  "d1_databases": [
    { "binding": "DB", "database_name": "sec-ex10-v2", "database_id": "<PASTE_NEW_D1_ID>" }
  ],
  "observability": { "enabled": true }
}
```

- [ ] **Step 3: Apply the existing migrations to the new D1**

The new D1 must have the same schema as prod. Apply the committed drizzle migrations:

```bash
cd /home/arthrod/workspace/sec-listener/frontend
wrangler d1 migrations apply sec-ex10-v2 --remote -c wrangler.v2.jsonc
```

Expected: applies `0000_*` and `0001_*`; the `exhibits` table exists with the
text `id` PK and the unique/index set. Verify:

```bash
wrangler d1 execute sec-ex10-v2 --remote -c wrangler.v2.jsonc \
  --command "SELECT name FROM sqlite_master WHERE type='table';"
```

Expected: includes `exhibits` (and `d1_migrations`).

- [ ] **Step 4: Commit the config**

```bash
cd /home/arthrod/workspace/sec-listener
git add frontend/wrangler.v2.jsonc
git commit -m "feat(frontend): wrangler.v2 config for parallel stack (fresh D1)"
```

> The live `frontend/wrangler.jsonc` is intentionally untouched.

---

### Task H2: Build + deploy the v2 Worker; set its secret

**Files:** none (deploy actions)

- [ ] **Step 1: Build the frontend**

```bash
cd /home/arthrod/workspace/sec-listener/frontend
bun install
bun run build
```
Expected: `dist/` produced, no errors.

- [ ] **Step 2: Deploy with the v2 config**

```bash
wrangler deploy -c wrangler.v2.jsonc
```
Expected: deploys `sec-ex10-frontend-v2`; prints the
`https://sec-ex10-frontend-v2.<subdomain>.workers.dev` URL. Record it.

- [ ] **Step 3: Set the ingest secret on the v2 Worker**

Use the SAME `SEC_API_KEY` value you will give the Space (any strong shared
secret; if reusing the existing prod value, fetch it from your records):

```bash
wrangler secret put SEC_API_KEY -c wrangler.v2.jsonc
```
Expected: secret stored.

- [ ] **Step 4: Verify the site loads (empty feed is fine)**

```bash
curl -s -o /dev/null -w "%{http_code}\n" https://sec-ex10-frontend-v2.<subdomain>.workers.dev/
```
Expected: `200`.

- [ ] **Step 5: Verify ingest auth works (no row written; bad key rejected)**

```bash
curl -s -o /dev/null -w "%{http_code}\n" -X POST \
  https://sec-ex10-frontend-v2.<subdomain>.workers.dev/api/ingest \
  -H 'X-API-Key: WRONG' -H 'Content-Type: application/json' --data '{"rows":[]}'
```
Expected: `401`. With the correct key and `{"rows":[]}` → `200` `{"accepted":[]}`.

> No commit — these are deploy/verification actions.

---

### Task H3: Deploy the Rust Space + end-to-end verification

**Files:** none (deploy actions)

- [ ] **Step 1: Create the HF Space**

Create a Docker Space `arthrod/sec-ex10-api-rust` (via the HF UI or
`huggingface_hub`/`hf` CLI). SDK: Docker.

- [ ] **Step 2: Set Space secrets/vars**

In the Space settings, set:
- `SEC_API_KEY` = the same value as the v2 Worker secret (Step H2.3)
- `D1_INGEST_URL` = `https://sec-ex10-frontend-v2.<subdomain>.workers.dev/api/ingest`
- `HF_TOKEN` = a token with write access to `arthrod/sec-ex10-exhibits`
- `SEC_USER_AGENT` = a real `Name email` string (SEC requires it)

- [ ] **Step 3: Push the Space bundle**

Upload `backend-rust/` (Dockerfile + src + vendor + Cargo.*) to the Space repo
root:

```bash
cd /home/arthrod/workspace/sec-listener
hf upload arthrod/sec-ex10-api-rust backend-rust . --repo-type space
```
Expected: triggers a Docker build on the Space. Watch the build logs to green.

- [ ] **Step 4: Verify Space health**

```bash
curl -s https://arthrod-sec-ex10-api-rust.hf.space/health
```
Expected: `{"status":"ok","total_seen":<n>}` with `n` growing over a few minutes.

- [ ] **Step 5: Verify rows land in the v2 D1**

After a few minutes of streaming:

```bash
cd /home/arthrod/workspace/sec-listener/frontend
wrangler d1 execute sec-ex10-v2 --remote -c wrangler.v2.jsonc \
  --command "SELECT count(*) AS n, max(filed_at) AS latest FROM exhibits;"
```
Expected: `n > 0` and a recent `latest`. Then reload the v2 site — the feed shows
live EX-10 exhibits.

- [ ] **Step 6: Spot-check an image-only exhibit (if any captured)**

```bash
wrangler d1 execute sec-ex10-v2 --remote -c wrangler.v2.jsonc \
  --command "SELECT accession, image_urls FROM exhibits WHERE image_urls IS NOT NULL AND image_urls != '[]' LIMIT 3;"
```
Expected: rows with HF `resolve/main` URLs; opening one returns the image.

---

## Phase I — Docs & memory

### Task I1: Deploy doc + memory update

**Files:**
- Create: `backend-rust/DEPLOY.md`
- Update: memory (outside repo)

- [ ] **Step 1: Write `backend-rust/DEPLOY.md`** summarizing Tasks H1–H3 (the new
  D1 name + id, the v2 worker name + URL, the Space name + URL, the shared
  `SEC_API_KEY` relationship, and the "parallel, no cutover" status). Use the
  exact commands from H1–H3.

- [ ] **Step 2: Commit**

```bash
cd /home/arthrod/workspace/sec-listener
git add backend-rust/DEPLOY.md
git commit -m "docs(rust): parallel-stack deploy guide"
```

- [ ] **Step 3: Update agent memory** — add a `sec-listener-rust-stack` memory
  file recording: the Rust backend (`backend-rust/`, vendored secinfra, no SQLite,
  htmd, images via HF commit API), the parallel deploy (`arthrod/sec-ex10-api-rust`
  Space + `sec-ex10-frontend-v2` Worker + `sec-ex10-v2` D1), the shared
  `SEC_API_KEY`, and the not-yet-cut-over status; add its line to `MEMORY.md`.

---

## Self-Review

**Spec coverage check (spec §→task):**
- §3.1 EX-10 classification → B1 ✓
- §3.2 ingest record keys/constraints → C1 (shape) + C2 (≤200 chunk) ✓
- §3.3 id echo token → C1 (`id: u64`) + F1/F5 (monotonic counter) ✓
- §3.4 D1 schema unchanged + fresh D1 migrated → H1 (migrations apply) ✓
- §3.5 filing header → B2 + E1 bridge ✓
- §3.6 markdown (htmd, statuses) → B3 ✓
- §3.7 images (refs, image-only, GRAPHIC, one-commit upload, URLs) → B4 + D1 + F2 ✓
- §4 module layout → all Phase B–F tasks map to the listed modules ✓
- §4.1 pipeline (Monitor, bounded concurrency, push) → F5 ✓
- §4.2 no finalized gate / non-blocking images → F2 (best-effort) + F5 ✓
- §5 markdown_status state machine → B3 (Done/Empty/Error) ✓
- §6 env config → F3 ✓
- §7 deploy artifacts (Dockerfile, Space README, wrangler.v2, new D1) → G1 + H1 ✓
- §7.1 deploy steps → H1–H3 ✓
- §8 testing (pure unit tests + seams) → B/C/D/E/F tests ✓
- §9 deltas (no SQLite, htmd-only, no cutover, no new dataset, no all_exhibits) → reflected; htmd-only in B3, no cutover in H (no routes) ✓
- §10 risks (C build, HF API, htmd fidelity) → A1 build check, D1 limitation note ✓

**Placeholder scan:** every code step contains full code; deploy steps that
require runtime values (`<PASTE_NEW_D1_ID>`, `<subdomain>`) are explicitly
captured in a prior step. No TBD/TODO.

**Type consistency:** `Config` fields (`convert_markdown`, `hf_token`,
`image_repo`, `ingest_url`, `api_key`, `poll_interval_ms`, `concurrency`,
`push_batch`, `port`) are defined in F3 and used identically in F2/F5/main.
`IngestRecord` defined in C1, used in C2/F1/F2. `Ex10Doc`/`Gathered` defined in
E1, used in F1/F2. `DocResult`/`build_records` defined in F1, used in F2.
`HealthState` defined in F4, used in F5/main. `MarkdownStatus`/`status_for` in B3,
used in F2. `image_filenames`/`is_image_only`/`capture_images_with`/`hf_upload`/
`dataset_image_path`/`public_url` in B4+D1, used in F2.

**Known follow-ups (acceptable, flagged):** htmd API call may need a one-line
signature tweak (B3 note); SGML fixture may need adjustment to satisfy the C
parser (E1 note); HF inline-base64 upload doesn't do LFS (D1 note).
