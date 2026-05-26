# SEC EX-10 Live — Frontend (Astro 6, fully live SSR)

Astro 6 app on **Cloudflare Workers**. Every page is **live SSR** — fetched from
the backend API at request time, cached at the edge (CDN + stale-while-revalidate).
**No rebuilds or redeploys** are needed when content changes; deploy only for code.

## Architecture

```
HF Space (FastAPI: listener + backfill + API, key-gated)
        ▲  request-time fetch (SEC_API_URL + SEC_API_KEY)
Cloudflare Worker (Astro SSR)  ──CDN / stale-while-revalidate──▶ users
```

- **Homepage** (`/`) — Live Content Collection: agreements in the last 60s, 60s auto-refresh.
- **Archive** (`/agreements/[page]`) — live SSR, paginated.
- **Detail** (`/agreement/[id]`) — live SSR via `getLiveEntry`; markdown → HTML.
- **Search** (`/search?q=`) — live full-text search against `/api/search` (replaced Pagefind).
- **Stats strip** — live `/api/stats`.

## Config (`astro:env`, server)

| Var | Meaning |
|-----|---------|
| `SEC_API_URL` | Backend base URL (the HF Space). Set in `wrangler.jsonc`. |
| `SEC_API_KEY` | API key (`X-API-Key`). Set as a Worker secret: `wrangler secret put SEC_API_KEY`. |

Local dev: put both in `.env`.

## Develop / deploy

```bash
bun install
bun run dev              # http://localhost:4321
bun run build            # fast — only /404 is static; everything else is SSR
bun run test:smoke       # BASE=<url> node test/smoke.mjs
wrangler deploy          # deploy code changes (content needs no redeploy)
```

Live: **https://sec-ex10-frontend.cicero-im.workers.dev**
