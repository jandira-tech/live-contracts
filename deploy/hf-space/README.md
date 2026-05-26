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
them, converts each to Markdown, and serves a read-only JSON API. Designed to sit
behind a **Cloudflare Tunnel** so the Astro frontend on Cloudflare Workers reads
it at request time (fully live, no rebuilds).

## What runs here

One process (`python -m sec_listener.worker`) = SEC listener + Markdown backfill
+ FastAPI API on port **7860**. Plus `cloudflared` when `TUNNEL_TOKEN` is set.

## Required Space secrets

| Secret | Purpose |
|--------|---------|
| `SEC_API_KEY` | API key clients must send as `X-API-Key`. |
| `TUNNEL_TOKEN` | Cloudflare Tunnel token (create the tunnel + route a hostname in the Cloudflare dashboard, then paste its token here). Optional — without it the API is reachable only via this Space's URL. |

## Endpoints

`/health`, `/api/ex10?page=&page_size=`, `/api/ex10/since?seconds=`,
`/api/ex10/{id}`, `/api/stats`.

## Notes

- The Space ships with a seed database for instant content; new filings stream in
  continuously. Enable persistent storage (mount at `/data` and set
  `SEC_DB_PATH=/data/ex10_listener.db`) to retain data across restarts.
- Public SEC filing data only. Not legal/investment advice.
