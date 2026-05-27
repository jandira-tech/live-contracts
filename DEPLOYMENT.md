# Deployment

Two halves: the **internal backend** (listener + backfill + key-gated API) and the
**Cloudflare Worker frontend** (the only public surface, live SSR at the edge). The
production backend runs as a **Hugging Face Docker Space**; the self-hosted tunnel
path below is an alternative.

```
SEC EDGAR ──> [worker: listener + markdown/metadata/image backfill] ──> SQLite
                                            │                              │
                                  [FastAPI API, X-API-Key]      HF dataset (mirror + boot-restore)
                                            │  request-time fetch
                                            ▼
                        [Astro frontend on Cloudflare Workers]  ──CDN/SWR──> live-contracts.arthur.law
```

## A. Backend — Hugging Face Space (production)

The Space bundle lives in `deploy/hf-space/` (Docker, port 7860). One process runs the
listener + backfill + API. See [deploy/hf-space/README.md](./deploy/hf-space/README.md).

Space secrets:

| Secret | Purpose |
|--------|---------|
| `SEC_API_KEY` | key clients send as `X-API-Key` (same value as the frontend's `SEC_API_KEY`) |
| `HF_TOKEN` | enables the dataset mirror + boot-restore + image capture |

HF Spaces have no persistent disk, so on boot the worker restores SQLite from the HF
dataset (`python -m sec_listener.boot_restore`) and re-mirrors it every
`SEC_HF_SYNC_INTERVAL` seconds. The Space ships with `seed.db` for instant first content.

## B. Backend — self-hosted (alternative)

```bash
uv pip install -e .
# Worker = continuous listener + backfill (supervise via cron/watchdog)
SEC_RUN_HOURS=0 python -m sec_listener.worker
# Internal API (bind localhost; set a strong key)
SEC_API_KEY="$(openssl rand -hex 24)" SEC_API_HOST=127.0.0.1 SEC_API_PORT=8799 \
  python -m sec_listener.api
```

The API binds `127.0.0.1` and requires `X-API-Key`. **Do not** expose the port directly —
front it with a **Cloudflare Tunnel** locked behind **Cloudflare Access** so only the Worker
can reach it:

```bash
cloudflared tunnel create sec-api
cloudflared tunnel route dns sec-api sec-api.internal.<your-domain>
# config.yml: ingress: [{ hostname: sec-api.internal.<your-domain>, service: http://127.0.0.1:8799 }, { service: http_status:404 }]
cloudflared tunnel run sec-api
```

## C. Frontend (Cloudflare Workers)

```bash
cd frontend
bun install && bun run build         # astro build (only /404 is static; everything else is SSR)
wrangler secret put SEC_API_KEY      # the key the API expects
# set SEC_API_URL in wrangler.jsonc -> the HF Space URL (or the tunnel hostname)
wrangler deploy
```

Live deployment: **https://live-contracts.arthur.law**

Every page is **live SSR** — fetched from the API at request time and cached at the edge.
There is no prerendered snapshot to refresh: content updates need **no rebuild or redeploy**;
deploy only for code changes. If the API is unreachable the pages degrade gracefully.

## Caching

The API sets `Cache-Control: ... stale-while-revalidate`; Cloudflare's CDN serves cached
responses instantly and revalidates in the background.
