# Deployment

Two halves: the **internal backend** (runs on your host, never public) and the
**Cloudflare Worker frontend** (public edge, serves prerendered pages + live SSR).

```
SEC EDGAR ──> [worker: listener + backfill] ──> SQLite (ex10_listener.db)
                                                     │
                                          [FastAPI internal API]  (127.0.0.1:8799, key-gated)
                                                     │  Cloudflare Tunnel (private)
                                                     ▼
                              [Astro frontend on Cloudflare Workers]  ──CDN/SWR──> users
```

## 1. Backend (host, private)

```bash
uv pip install -e .
# Worker = continuous listener + markdown backfill (supervised by watchdog.sh via cron)
SEC_RUN_HOURS=0 python -m sec_listener.worker
# Internal API (bind localhost; set a strong key)
SEC_API_KEY="$(openssl rand -hex 24)" SEC_API_HOST=127.0.0.1 SEC_API_PORT=8799 \
  python -m sec_listener.api
```

The API binds `127.0.0.1` and requires `X-API-Key`. **Do not** expose the port.

## 2. Private link: Cloudflare Tunnel

Expose the API to *only* your Worker (no public port):

```bash
cloudflared tunnel create sec-api
cloudflared tunnel route dns sec-api sec-api.internal.<your-domain>
# config.yml:  ingress: [{ hostname: sec-api.internal.<your-domain>, service: http://127.0.0.1:8799 }, { service: http_status:404 }]
cloudflared tunnel run sec-api
```

Lock the tunnel hostname behind **Cloudflare Access** (service token) so only the
Worker can reach it.

## 3. Frontend (Cloudflare Workers)

```bash
cd frontend
bun install && bun run build        # astro build
wrangler kv namespace create SESSION # once; put id in wrangler.jsonc
wrangler secret put SEC_API_KEY      # the key the API expects
# set SEC_API_URL in wrangler.jsonc -> https://sec-api.internal.<your-domain>
wrangler deploy
```

Live deployment: **https://sec-ex10-frontend.cicero-im.workers.dev**

- Prerendered (archive/detail/search) work from the build snapshot regardless of
  the API. The live homepage feed needs the tunnel + `SEC_API_KEY` set; until then
  it degrades gracefully ("temporarily unavailable").
- Re-run `bun run build && wrangler deploy` to refresh the prerendered snapshot
  (e.g. via cron) so the static archive stays current.

## Caching

The API sets `Cache-Control: ... stale-while-revalidate`; Cloudflare's CDN serves
cached responses instantly and revalidates in the background.
