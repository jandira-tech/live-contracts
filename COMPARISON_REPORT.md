# Rust vs Python (standard) backend — live comparison

Window start: 2026-06-04T16:27:13Z. Both backends watch the live SEC EDGAR feed
(Python via RSS, Rust via EFTS) and emit one record per EX-10 exhibit.

## Setup
- **Python (standard)**: `python -m sec_listener.worker` → writes `ex10_exhibits` in `ex10_listener.db`.
- **Rust**: `backend-rust/target/release/sec-ex10-rust` → POSTs ingest records to a
  **local capture sink** (`scripts/ingest_sink.py` on :8092 → `rust_ingest_capture.jsonl`),
  NOT production D1, so nothing is double-written upstream. `HF_TOKEN` unset (no image
  uploads), matching the Python worker's env.

## Discovery (different windows, expected)
| | EX-10 docs | filings |
|---|---|---|
| Rust (recent EFTS backfill) | 37 | 18 |
| Python (multi-day RSS history) | 119 | 49 |

Overlap is small at the start because Python has days of accumulated history and Rust
re-scanned only the recent EFTS window. Overlap grows as both run forward on new filings.

## Field parity on overlapping docs (accession+doc_type+filename)
On the overlapping docs:
- **cik, form_type, description, sequence, markdown_status** — identical.
- **markdown** — within 1–2% length (essentially same content; different HTML→MD converters).
- **filing_url** — **DIFFERS** (see below).

## The one real discrepancy: `filing_url`
- Rust: `.../data/{cik}/{accession_no_dashes}/{accession_dashed}.txt` (with accession subfolder)
- Python: `.../data/{cik}/{accession_dashed}.txt` (flat legacy path)

Both return HTTP 200, but the strings differ — so D1 would hold inconsistent `filing_url`
values depending on which backend ingested the row. The two should agree on one form.

`found_at` also differs in format (Rust: RFC3339 + nanoseconds + TZ; Python: naive
`YYYY-MM-DD HH:MM:SS`) — legitimately different detection timestamps, but worth normalizing.

## Side finding: Python fd leak (was already down before this run)
The pre-existing Python worker (PID 136906, up 5 days) had wedged since ~06:58 with
`OperationalError: unable to open database file` / `Too many open files` — 1015/1024 fds
were unclosed handles to `ex10_listener.db`. Cause: `with sqlite3.connect() as conn:`
commits but does NOT close the connection; fds rely on GC. Under the 1024 soft cap a
backfill burst outpaced GC, pegged the limit, and never recovered.

Mitigation applied: relaunched with `ulimit -n 65536`; GC now bulk-reclaims (observed
fds oscillate, e.g. 5150 → 148) and the process stays healthy. Proper fix (separate PR):
wrap connections in `contextlib.closing(...)` / explicit `conn.close()` in `db.py`.
