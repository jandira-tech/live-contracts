# /// script
# requires-python = ">=3.10"
# dependencies = ["duckdb>=1.1.0", "httpx"]
# ///
"""One-off: copy the HF parquet content table into D1 via /api/ingest."""
import os, json, duckdb, httpx

INGEST = os.environ.get("D1_INGEST_URL", "https://live-contracts.arthur.law/api/ingest")
KEY = os.environ["SEC_API_KEY"]
P = "hf://datasets/arthrod/sec-ex10-exhibits/data/exhibits.parquet"

con = duckdb.connect()
con.execute(f"CREATE SECRET hf (TYPE huggingface, TOKEN '{os.environ['HF_TOKEN']}');")
cols = ["id","accession","cik","form_type","doc_type","filename","description","sequence",
        "filing_url","found_at","markdown_status","filing_metadata","image_urls","markdown"]
rows = con.execute(f"SELECT {', '.join(cols)} FROM '{P}'").fetchall()

def rec(r):
    d = dict(zip(cols, r))
    meta = d.get("filing_metadata") or ""
    try: d["filed_at"] = (json.loads(meta).get("filed_at") or "") if meta else ""
    except Exception: d["filed_at"] = ""
    return d

recs = [rec(r) for r in rows]
sent = 0
with httpx.Client(timeout=120) as client:
    for i in range(0, len(recs), 100):
        resp = client.post(INGEST, headers={"X-API-Key": KEY}, json={"rows": recs[i:i+100]})
        resp.raise_for_status()
        sent += len(resp.json()["accepted"]); print(f"  {sent}/{len(recs)}")
print(f"done: {sent} rows")
