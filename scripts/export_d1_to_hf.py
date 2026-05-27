# /// script
# requires-python = ">=3.10"
# dependencies = ["pyarrow", "huggingface_hub"]
# ///
"""Export the full D1 `exhibits` table to data/exhibits.parquet on HF (public mirror).

Reads D1 through `wrangler d1 execute --remote --json` in id-keyset pages, so it uses
the machine's existing `wrangler login` — **no Cloudflare API token required** for a
manual/local run. (For an unattended GitHub Action / cron, wrangler authenticates via a
CLOUDFLARE_API_TOKEN env var scoped `D1:Read` — the one place a token is still needed.)

Run from the repo root (needs `frontend/wrangler.jsonc` + `cd frontend && bun install`):
    HF_TOKEN=... uv run scripts/export_d1_to_hf.py
"""
import json
import os
import subprocess
import tempfile

import pyarrow as pa
import pyarrow.parquet as pq
from huggingface_hub import HfApi

DB = "sec-ex10"
REPO = "arthrod/sec-ex10-exhibits"
PAGE = 100  # markdown bodies avg ~67KB; keep each D1 response well under its size cap
FRONTEND = os.path.join(os.path.dirname(__file__), "..", "frontend")


def d1(sql: str) -> list[dict]:
    """Run a read query via wrangler (existing login) and return the result rows."""
    out = subprocess.run(
        ["npx", "wrangler", "d1", "execute", DB, "--remote", "--json", "--command", sql],
        capture_output=True, text=True, cwd=FRONTEND, check=True,
    ).stdout
    start = out.find("[")  # skip any wrangler banner before the JSON payload
    if start < 0:
        raise RuntimeError(f"no JSON in wrangler output: {out[:200]}")
    return json.loads(out[start:])[0]["results"]


def main() -> None:
    token = os.environ["HF_TOKEN"]  # fail fast if missing
    rows: list[dict] = []
    last = 0
    while True:
        page = d1(f"SELECT * FROM exhibits WHERE id > {last} ORDER BY id LIMIT {PAGE}")
        if not page:
            break
        rows.extend(page)
        last = page[-1]["id"]
        print(f"  fetched {len(rows)}")
    if not rows:
        print("no rows in D1; nothing to export")
        return

    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "exhibits.parquet")
        pq.write_table(pa.Table.from_pylist(rows), path)
        HfApi(token=token).upload_file(
            path_or_fileobj=path, path_in_repo="data/exhibits.parquet",
            repo_id=REPO, repo_type="dataset",
        )
    print(f"exported {len(rows)} rows to {REPO}/data/exhibits.parquet")


if __name__ == "__main__":
    main()
