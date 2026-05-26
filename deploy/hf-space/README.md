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

One process (`python -m sec_listener.worker`) = SEC listener + Markdown backfill
+ FastAPI API on port **7860**, which Hugging Face exposes at the Space URL.

## Required Space secret

| Secret | Purpose |
|--------|---------|
| `SEC_API_KEY` | Key clients must send as the `X-API-Key` header. Set it, and use the same value in the frontend's `SEC_API_KEY`. |

## Endpoints

`/health` (open), `/api/ex10?page=&page_size=`, `/api/ex10/since?seconds=`,
`/api/ex10/{id}`, `/api/stats`, `/api/search?q=`.

## Notes

- Ships with a seed database for instant content; new filings stream in
  continuously. Enable persistent storage (mount `/data`, set
  `SEC_DB_PATH=/data/ex10_listener.db`) to retain data across restarts.
- Cloudflare Tunnel is **not** used — Hugging Face bans `cloudflared` on Spaces,
  and HF already provides the public HTTPS endpoint.
- Public SEC filing data only. Not legal/investment advice.
