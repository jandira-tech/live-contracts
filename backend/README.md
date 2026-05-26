# SEC EX-10 Listener — Backend

Hardened, continuously-parsing listener for SEC EX-10 (material contract) exhibits,
with HTML→Markdown conversion and an internal read-only API.

## Layout

```
backend/sec_listener/
  config.py      # env-driven Config
  net.py         # retry_async (exponential backoff)
  parsing.py     # pure RSS parse + EX-10 classification
  converter.py   # HTML/PDF/text -> Markdown via markitdown
  db.py          # SQLite data-access layer (+ markdown column migration)
  listener.py    # orchestration + live polling loop
  api.py         # FastAPI internal API (PR2)
  worker.py      # continuous worker + backfill (PR3)
backend/tests/   # pytest suite
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
