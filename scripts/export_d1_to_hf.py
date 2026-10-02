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


def parse_wrangler_json(out: str) -> list[dict]:
    """The result rows from `wrangler d1 execute --json`, skipping any banner text
    before the JSON array (banners can contain brackets, e.g. "[wrangler]")."""
    decoder = json.JSONDecoder()
    for i, ch in enumerate(out):
        if ch != "[":
            continue
        try:
            value, _ = decoder.raw_decode(out, i)
        except json.JSONDecodeError:
            continue
        if isinstance(value, list) and value and isinstance(value[0], dict) and "results" in value[0]:
            return value[0]["results"]
    raise RuntimeError(f"no JSON result in wrangler output: {out[:200]}")


def fetch_all(d1_fn, page: int = PAGE) -> list[dict]:
    """Keyset-page the table by id. Ids are text (UUIDv7, plus a few legacy values),
    so the bound is a quoted string compared as text."""
    rows: list[dict] = []
    last = ""
    while True:
        bound = last.replace("'", "''")
        batch = d1_fn(f"SELECT * FROM exhibits WHERE id > '{bound}' ORDER BY id LIMIT {page}")
        if not batch:
            return rows
        rows.extend(batch)
        last = str(batch[-1]["id"])
        print(f"  fetched {len(rows)}")


def d1(sql: str) -> list[dict]:
    """Run a read query via wrangler (existing login) and return the result rows."""
    out = subprocess.run(
        ["npx", "wrangler", "d1", "execute", DB, "--remote", "--json", "--command", sql],
        capture_output=True, text=True, cwd=FRONTEND, check=True,
    ).stdout
    return parse_wrangler_json(out)


def main() -> None:
    token = os.environ["HF_TOKEN"]  # fail fast if missing
    rows = fetch_all(d1)
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
