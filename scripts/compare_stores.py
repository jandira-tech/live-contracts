#!/usr/bin/env python3
"""Compare the Rust store (stateful mode) against the Python listener's store.

Both write SQLite with the same schema, so this diffs them table-for-table by
(accession, doc_type, filename). Use after starting both fresh from a common t0.

  rust : rust_ex10.db        (SEC_STORE_PATH)
  py   : ex10_listener.db     (Python worker)

Run:  .venv/bin/python scripts/compare_stores.py
"""
import sqlite3, pathlib, datetime as dt

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUST_DB = ROOT / "rust_ex10.db"
PY_DB = ROOT / "ex10_listener.db"


def load(db, table):
    if not db.exists():
        return {}
    con = sqlite3.connect(db)
    con.row_factory = sqlite3.Row
    try:
        return {
            (r["accession"], r["doc_type"], r["filename"]): dict(r)
            for r in con.execute(f"SELECT * FROM {table}")
        }
    finally:
        con.close()


t0 = (ROOT / "comparison_t0.txt").read_text().strip() if (ROOT / "comparison_t0.txt").exists() else "?"
print(f"common t0: {t0}    now: {dt.datetime.now(dt.UTC):%Y-%m-%d %H:%M:%S}Z")
print("=" * 66)

for table in ("ex10_exhibits", "all_exhibits"):
    r = load(RUST_DB, table)
    p = load(PY_DB, table)
    overlap = set(r) & set(p)
    r_only, p_only = set(r) - set(p), set(p) - set(r)
    r_acc = len({k[0] for k in r})
    p_acc = len({k[0] for k in p})
    print(f"\n## {table}")
    print(f"  Rust  : {len(r):>5} docs / {r_acc:>4} filings")
    print(f"  Python: {len(p):>5} docs / {p_acc:>4} filings")
    print(f"  overlap {len(overlap)} | rust-only {len(r_only)} | python-only {len(p_only)}")
    if overlap and table == "ex10_exhibits":
        fields = ["cik", "form_type", "description", "sequence", "filing_url", "markdown_status"]
        mism = {f: 0 for f in fields}
        md_close = 0
        for k in overlap:
            for f in fields:
                if str(r[k].get(f) or "") != str(p[k].get(f) or ""):
                    mism[f] += 1
            la = len(r[k].get("markdown") or "")
            lb = len(p[k].get("markdown") or "")
            if max(la, lb) == 0 or min(la, lb) / max(la, lb) >= 0.8:
                md_close += 1
        print("  field parity on overlap:")
        for f in fields:
            print(f"    {f:<16}: {'OK' if mism[f]==0 else str(mism[f])+' differ'}")
        print(f"    markdown length : {md_close}/{len(overlap)} within 20%")
    if p_only and table == "ex10_exhibits":
        print("  sample python-only (Rust missed — e.g. 429-throttled):")
        for k in sorted(p_only)[:4]:
            print(f"    {k[0]} {k[1]} {k[2]}")
