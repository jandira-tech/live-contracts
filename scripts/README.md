# scripts

Operational scripts run from a **trusted host** (GitHub Action / cron) — never the public Cloudflare Worker.

## `export_d1_to_hf.py` — D1 → HF parquet public export (phase 2)

Exports the full D1 `exhibits` table to `data/exhibits.parquet` in the public HF dataset
`arthrod/sec-ex10-exhibits`. Reads D1 through **`wrangler d1 execute --remote --json`** in
id-keyset pages (`WHERE id > ? ORDER BY id LIMIT 100`), writes a parquet file, and uploads it
with `huggingface_hub`.

Run from the repo root (needs `frontend/wrangler.jsonc`; run `cd frontend && bun install` once):

```bash
HF_TOKEN=... uv run scripts/export_d1_to_hf.py
```

(PEP-723 inline metadata declares deps: `pyarrow`, `huggingface_hub`.)

### Credentials

Because it shells out to `wrangler`, a **manual / local run needs no Cloudflare API token** —
it uses your existing `wrangler login`.

| Var | Purpose |
| --- | --- |
| `HF_TOKEN` | Hugging Face token with write access to `arthrod/sec-ex10-exhibits`. |

For an **unattended GitHub Action / cron** there's no interactive login, so wrangler reads
`CLOUDFLARE_API_TOKEN` from the env — mint that one in the Cloudflare dashboard scoped **`D1:Read`**
(wrangler cannot create tokens). That is the only place a Cloudflare token is needed; the public
Worker never holds one.

> This is a *mirror refresh* — running it when the HF parquet already matches D1 just re-uploads
> identical content. It earns its keep once D1 has grown past the mirror.
