---
title: SEC EX-10 Listener (Rust)
emoji: 📄
colorFrom: blue
colorTo: indigo
sdk: docker
app_port: 7860
pinned: false
---

# SEC EX-10 Listener — Rust Backend

Streams SEC filings via the `secinfra` crate, filters to traditional EX-10 exhibits,
converts HTML to Markdown, captures exhibit images to a Hugging Face dataset,
and POSTs structured records to the Astro frontend's `/api/ingest` endpoint.

## Configuration (environment variables)

| Variable | Default | Description |
|---|---|---|
| `SEC_API_KEY` | (required) | API key for the ingest endpoint |
| `SEC_INGEST_URL` | `https://live-contracts.arthur.law/api/ingest` | Ingest POST target |
| `SEC_POLL_INTERVAL_MS` | `200` | Monitor polling interval |
| `SEC_CONCURRENCY` | `8` | Max concurrent submission processing |
| `SEC_PUSH_BATCH` | `100` (max 200) | Rows per ingest POST |
| `SEC_CONVERT_MARKDOWN` | `true` | Set `false` to skip HTML→MD |
| `SEC_IMAGE_REPO` | `arthrod/sec-ex10-exhibits` | HF dataset for exhibit images |
| `HF_TOKEN` | (optional) | Hugging Face API token for image uploads |
| `PORT` | `7860` | Health server port |
| `RUST_LOG` | `info` | Tracing filter |

## Health

`GET /health` returns `{"status":"ok","total_seen":N}`.
