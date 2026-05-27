---
title: SEC EX-10 Backend
emoji: 📄
colorFrom: blue
colorTo: green
sdk: docker
app_port: 7860
pinned: false
short_description: Live SEC EX-10 material-contract listener + API
---

# SEC EX-10 Backend

Continuously polls SEC EDGAR for **EX-10 (material contract)** exhibits, extracts
them, converts each to Markdown, and serves a read-only JSON API. The Astro
frontend on Cloudflare Workers reads this API at request time (fully live, no
rebuilds) and caches responses at the edge (CDN + stale-while-revalidate).

## What runs here

One process (`python -m sec_listener.worker`) = SEC listener + Markdown / filing-header /
image backfill + FastAPI API on port **7860**, which Hugging Face exposes at the Space URL.

## Space secrets

| Secret | Purpose |
|--------|---------|
| `SEC_API_KEY` | Key clients must send as the `X-API-Key` header. Set it, and use the same value in the frontend's `SEC_API_KEY`. |
| `HF_TOKEN` | Enables the parallel HF-dataset mirror, cold-start boot-restore, and scanned-exhibit image capture. |

## Endpoints

`/health` (open), `/api/ex10?page=&page_size=&form=&cik=&filer=&sort=`,
`/api/ex10/since?seconds=`, `/api/ex10/{id}`, `/api/facets`, `/api/stats`, `/api/search?q=`.

## Notes

- Ships with a seed database for instant content; new filings stream in continuously.
- HF Spaces have **no persistent disk**, so durability comes from the HF dataset mirror:
  on boot the worker restores SQLite from `arthrod/sec-ex10-exhibits`
  (`python -m sec_listener.boot_restore`) and re-mirrors every `SEC_HF_SYNC_INTERVAL` seconds.
- Public SEC filing data only. Not legal/investment advice. Not affiliated with the SEC.
