# scripts

Operational scripts run from a **trusted host** (GitHub Action / cron) — never the public Cloudflare Worker.

## `export_d1_to_hf.py` — D1 → HF parquet public export (phase 2)

Exports the full D1 `exhibits` table to `data/exhibits.parquet` in the public HF dataset
`arthrod/sec-ex10-exhibits`. Reads D1 via the Cloudflare D1 HTTP API in id-keyset pages
(`WHERE id > ? ORDER BY id LIMIT 1000`), writes a parquet file, and uploads it with `huggingface_hub`.

Run:

```bash
uv run scripts/export_d1_to_hf.py
```

(PEP-723 inline metadata declares deps: `httpx`, `pyarrow`, `huggingface_hub`.)

### Environment variables

| Var | Purpose |
| --- | --- |
| `CF_ACCOUNT_ID` | Cloudflare account ID. |
| `CF_D1_DATABASE_ID` | D1 database ID — `dfb55595-bebe-4d5f-80e6-658538ad3da8`. |
| `CF_API_TOKEN` | Cloudflare API token scoped **`D1:Read`** only. |
| `HF_TOKEN` | Hugging Face token with write access to `arthrod/sec-ex10-exhibits`. |

### Security note

This script is the **only** place a Cloudflare API token is used, and that token is
**read-only** (`D1:Read`). It must be run only from a trusted host (GitHub Action / cron),
**never** from the public Worker. The public Worker never holds a Cloudflare API token.
