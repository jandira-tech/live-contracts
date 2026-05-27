# Live Contracts — Frontend (Astro 6, fully live SSR)

Astro 6 app on **Cloudflare Workers**, served at **[live-contracts.arthur.law](https://live-contracts.arthur.law)**.
Every page is **live SSR** — fetched from the backend API at request time, cached at the edge
(CDN + stale-while-revalidate). **No rebuilds or redeploys** are needed when content changes;
deploy only for code.

## Architecture

```
HF Space (FastAPI: listener + backfill + API, key-gated)
        ▲  request-time fetch (SEC_API_URL + SEC_API_KEY)
Cloudflare Worker (Astro SSR)  ──CDN / stale-while-revalidate──▶ users
```

- **Homepage** (`/`) — Live Content Collection: agreements in the last 60s, 60s auto-refresh.
- **Browse** (`/agreements/[page]`) — live SSR, paginated, with a faceted sidebar (filing type via
  `/api/facets`, filer/CIK filters, newest/oldest sort). Ordering is by actual **filing time**.
- **Detail** (`/agreement/[id]`) — live SSR via `getLiveEntry`; markdown → HTML; captured exhibit images.
- **Search** (`/search?q=`) — live full-text search against `/api/search` (replaced Pagefind).
- **Stats strip** — live `/api/stats`.

## Design system

Light editorial-technical theme (tokens in `src/styles/global.css`). Two font families only:
**Manrope** (body, Google Fonts) and self-hosted **Departure Mono** (mono, `public/fonts/`). The
structural yellow accent is never used as text color. `bun run test:fonts` guards the font wiring.

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
bun run test:fonts       # font-loading guard (no server needed)
bun run test:smoke       # BASE=<url> node test/smoke.mjs
wrangler deploy          # deploy code changes (content needs no redeploy)
```

Live: **https://live-contracts.arthur.law**
