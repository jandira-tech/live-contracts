# Deployment

Two halves: the **backend ingestion worker** (listener + backfill, running on a Hugging Face
Docker Space) and the **Cloudflare Worker frontend** (the only public surface, live SSR at the
edge). **Cloudflare D1** is the durable, authoritative store; the frontend reads it directly via
the `DB` binding, and the backend pushes *finalized* rows to it through `POST /api/ingest`.

```
SEC EDGAR ──> [HF Space worker: listener + markdown/metadata/image backfill] ──> working SQLite
                                            │  finalized rows → POST /api/ingest (X-API-Key)
                                            ▼
                        [Astro Worker] ──UPSERT──▶ Cloudflare D1 (authoritative)
                                            ▲  reads via Drizzle binding
                        live-contracts.arthur.law ◀── CDN/SWR
```

## A. Backend — Hugging Face Space (production)

The Space bundle lives in `deploy/hf-space/` (Docker, port 7860). One process runs the
listener + backfill, pushing finalized rows to D1. See [deploy/hf-space/README.md](./deploy/hf-space/README.md).

Space secrets:

| Secret | Purpose |
|--------|---------|
| `SEC_API_KEY` | sent as `X-API-Key` to the Worker's `/api/ingest` (same value as the Worker's `SEC_API_KEY` secret) |
| `HF_TOKEN` | enables scanned-exhibit image capture → HF + the phase-2 dataset export |

D1 is durable, so there is **no boot-restore** — the Space's SQLite is just a working buffer; on
restart it reseeds from `seed.db` and re-pushes (idempotent UPSERT). Set `D1_INGEST_URL` (defaults
to `https://live-contracts.arthur.law/api/ingest`).

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
