#!/usr/bin/env python3
"""Compare the Rust backend (RSS+EFTS) against the Python listener.

Rust output   = rust_ingest_capture.jsonl  (records POSTed to the local sink)
Python output = ex10_exhibits rows in ex10_listener.db

Apples-to-apples: Rust's startup EFTS backfill re-emits recent *history*, so we
bound the comparison to the span of filing times Rust actually captured
(filed_at lo..hi) and compare against the Python EX-10 rows filed in that same
span. That isolates "for the filings both could have seen, do they agree?" from
Python's deep pre-existing history.

Run:  .venv/bin/python scripts/compare_backends.py
"""
import json, sqlite3, datetime as dt, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent


def parse_filed(s):
    try:
        return dt.datetime.strptime((s or "")[:14], "%Y%m%d%H%M%S")
    except Exception:
        return None


def py_filed(row):
    m = row.get("filing_metadata")
    if m:
        try:
            return parse_filed(json.loads(m).get("filed_at"))
        except Exception:
            return None
    return None


# --- Rust captured records ---
rust = {}
cap = ROOT / "rust_ingest_capture.jsonl"
if cap.exists():
    for line in cap.read_text().splitlines():
        if line.strip():
            r = json.loads(line)
            rust[(r["accession"], r["doc_type"], r["filename"])] = r

rust_filed = sorted(f for f in (parse_filed(v.get("filed_at")) for v in rust.values()) if f)
lo, hi = (rust_filed[0], rust_filed[-1]) if rust_filed else (None, None)

# --- Python rows, bounded to Rust's filing-time span ---
con = sqlite3.connect(ROOT / "ex10_listener.db"); con.row_factory = sqlite3.Row
py_all = {}
py_span = {}
for r in con.execute("SELECT * FROM ex10_exhibits"):
    d = dict(r); key = (d["accession"], d["doc_type"], d["filename"])
    py_all[key] = d
    f = py_filed(d)
    if lo and hi and f and lo <= f <= hi:
        py_span[key] = d

now = dt.datetime.now(dt.UTC).strftime("%Y-%m-%d %H:%M:%S")
print(f"now (UTC): {now}")
print(f"Rust capture filing span: {lo} .. {hi}")
print("=" * 64)
print(f"Rust captured        : {len(rust):>4} EX-10 docs / {len({k[0] for k in rust}):>3} filings")
print(f"Python (all history) : {len(py_all):>4} EX-10 docs")
print(f"Python (in Rust span): {len(py_span):>4} EX-10 docs / {len({k[0] for k in py_span}):>3} filings")

ov = set(rust) & set(py_span)
rust_only = set(rust) - set(py_all)          # Rust found, Python doesn't have at all
py_only = set(py_span) - set(rust)            # in-span filings Python has but Rust missed
print("-" * 64)
print(f"Agreement (Rust ∩ Python-in-span)        : {len(ov)} docs")
print(f"Rust docs NOT in Python's full history    : {len(rust_only)}")
print(f"In-span filings Python has but Rust missed : {len(py_only)}")
pct = (100 * len(set(rust) & set(py_all)) // len(rust)) if rust else 0
print(f"Rust docs also present in Python (anywhere): {pct}%")

if ov:
    fields = ["cik", "form_type", "description", "sequence", "filing_url", "markdown_status"]
    mism = {f: 0 for f in fields}
    for k in ov:
        for f in fields:
            if str(rust[k].get(f) or "") != str(py_span[k].get(f) or ""):
                mism[f] += 1
    print("-" * 64)
    print(f"field parity on {len(ov)} overlapping docs:")
    for f in fields:
        print(f"  {f:<16}: {'OK' if mism[f]==0 else str(mism[f]) + ' differ'}")

if py_only:
    print("-" * 64)
    print("sample of in-span filings Python caught that Rust didn't:")
    for k in sorted(py_only)[:5]:
        print(f"  {k[0]} {k[1]} {k[2]}")
