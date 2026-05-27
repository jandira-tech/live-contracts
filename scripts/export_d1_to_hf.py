# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx", "pyarrow", "huggingface_hub"]
# ///
"""Export the full D1 exhibits table to data/exhibits.parquet on HF (public mirror).
Reads D1 via the Cloudflare D1 HTTP API (a READ-ONLY token) in id-keyset pages.
Run from a trusted host (GitHub Action / cron), never the public Worker."""
import os, tempfile
import httpx, pyarrow as pa, pyarrow.parquet as pq
from huggingface_hub import HfApi

ACCT = os.environ["CF_ACCOUNT_ID"]; DBID = os.environ["CF_D1_DATABASE_ID"]
CF_TOKEN = os.environ["CF_API_TOKEN"]  # D1:Read
REPO = "arthrod/sec-ex10-exhibits"
URL = f"https://api.cloudflare.com/client/v4/accounts/{ACCT}/d1/database/{DBID}/query"

def query(sql, params):
    r = httpx.post(URL, headers={"Authorization": f"Bearer {CF_TOKEN}"},
                   json={"sql": sql, "params": params}, timeout=120)
    r.raise_for_status()
    return r.json()["result"][0]["results"]

rows, last = [], 0
while True:
    page = query("SELECT * FROM exhibits WHERE id > ? ORDER BY id LIMIT 1000", [last])
    if not page: break
    rows.extend(page); last = page[-1]["id"]; print(f"  fetched {len(rows)}")

with tempfile.TemporaryDirectory() as d:
    path = os.path.join(d, "exhibits.parquet")
    pq.write_table(pa.Table.from_pylist(rows), path)
    HfApi(token=os.environ["HF_TOKEN"]).upload_file(
        path_or_fileobj=path, path_in_repo="data/exhibits.parquet", repo_id=REPO, repo_type="dataset")
print(f"exported {len(rows)} rows")
