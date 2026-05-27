# Live Contracts — Backend

Hardened, continuously-parsing listener for SEC EX-10 (material contract) exhibits,
with HTML→Markdown conversion, filing-header extraction, scanned-exhibit image capture,
a read-only API, and an optional Hugging Face dataset mirror.

## Layout

```
backend/sec_listener/
  config.py        # env-driven Config
  net.py           # retry_async (exponential backoff)
  parsing.py       # pure RSS parse + EX-10 classification + filing-header extraction
  converter.py     # HTML/PDF/text -> Markdown via markitdown
  db.py            # SQLite data-access layer (markdown/metadata/image columns + facets)
  listener.py      # orchestration + live polling loop
  api.py           # FastAPI internal API (feed, facets, search, detail, stats)
  worker.py        # continuous worker: markdown + metadata + image backfill + D1 push loop
  images.py        # scanned-exhibit image capture -> HF dataset (bulk single-commit)
  d1_sync.py       # push *finalized* rows to Cloudflare D1 via the Worker's /api/ingest
  hf_sync.py       # parquet helpers reused by the phase-2 D1 -> HF public export
backend/tests/     # pytest suite
```

## Configuration (env vars)

| Var | Default | Meaning |
|-----|---------|---------|
| `SEC_DB_PATH` | `ex10_listener.db` | SQLite path |
| `SEC_POLL_INTERVAL` | `60` | seconds between RSS polls |
| `SEC_RUN_HOURS` | `24` | stop after N hours (`0` = run forever) |
| `SEC_RPS` | `5` | max requests/sec to SEC (limit is 10) |
| `SEC_CONVERT_MARKDOWN` | `true` | convert exhibit HTML to Markdown |
| `SEC_USER_AGENT` | `SEC EX-10 Listener ...` | required SEC UA header |
| `SEC_SERVE_API` | `false` | also serve the FastAPI API in-process |
| `SEC_API_HOST` / `SEC_API_PORT` | `127.0.0.1` / `8799` | API bind address (localhost only) |
| `SEC_API_KEY` | — | required `X-API-Key` for all `/api/*` routes |
| `HF_TOKEN` | — | enables the HF dataset mirror + image capture (opt-in) |
| `HF_DATASET_REPO` | `arthrod/sec-ex10-exhibits` | dataset to mirror to / restore from |
| `SEC_HF_SYNC_INTERVAL` | `900` | seconds between dataset snapshots |

## API (read-only, `X-API-Key`)

`/health` (open), `/api/ex10` (paginated; `form`/`cik`/`filer`/`sort` filters),
`/api/ex10/since?seconds=`, `/api/ex10/{id}`, `/api/facets`, `/api/search?q=`, `/api/stats`.
The feed is ordered by actual **filing time** (`filing_metadata.filed_at`, expression-indexed).

## D1 sync (authoritative store)

Local SQLite is just a working buffer. When a row is **finalized** (markdown terminal +
filing_metadata present + image_urls resolved), `d1_sync` POSTs it once to the Astro Worker's
`/api/ingest` (authed with `SEC_API_KEY`), which UPSERTs it into **Cloudflare D1** — the durable,
authoritative store the frontend reads. Idempotent, so a Space restart (which reseeds from
`seed.db`) just re-pushes harmlessly; no boot-restore needed. The HF dataset is a phase-2 public
export from D1 (`scripts/export_d1_to_hf.py`); images still go to HF when `HF_TOKEN` is set.

## Run

```bash
uv pip install -e .
sec-listener         # or: python -m sec_listener.listener
```

## Robustness

- Exponential-backoff retries on RSS fetches (`net.retry_async`); a 503 page is
  retried then skipped without stopping the loop.
- Every filing is processed in its own try/except; one bad submission can't halt
  parsing. Accessions are marked *seen* up-front so a broken filing isn't retried forever.
- Markdown conversion failures degrade gracefully to empty markdown (`markdown_status`).
- Blocking datamule work runs in a thread so the event loop stays responsive.

## Tests

```bash
PYTHONPATH=backend python -m pytest backend/tests/
```
