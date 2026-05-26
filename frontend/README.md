# SEC EX-10 Live — Frontend (Astro 6, hybrid SSR)

Astro 6 app deployed to **Cloudflare Workers**. Hybrid: the live feed is
server-rendered at request time; the archive, detail pages and search index are
prerendered for speed + Pagefind.

## Architecture

- **Live homepage** (`/`) — `getLiveCollection('agreements', { filter: { seconds: 60 } })`
  via a **Live Content Collection** loader (`src/live.config.ts` + `src/loaders/sec-api.ts`).
  Fetches at request time; auto-refreshes every 60s (`<meta refresh>`).
- **Archive** (`/agreements/[page]`) — prerendered with Astro's native `paginate()`.
- **Detail** (`/agreement/[id]`) — prerendered; markdown rendered to HTML.
- **Search** (`/search`) — **Pagefind** full-text search over the prerendered pages.
- **Edge caching** — `Cache-Control: ... stale-while-revalidate` set at the API
  origin; the Cloudflare CDN serves cached responses instantly while revalidating.

## Data source

All data comes from the internal FastAPI API (`SEC_API_URL`, default
`http://127.0.0.1:8799`). The API is **private** — the Worker reaches it over a
Cloudflare Tunnel / private network carrying `SEC_API_KEY`. Never public.

Env is declared via `astro:env` (`astro.config.ts`): `SEC_API_URL`, `SEC_API_KEY`
(both server secrets).

## Develop / build / deploy

```bash
pnpm install
echo 'SEC_API_URL=http://127.0.0.1:8799' > .env   # local
pnpm dev                                           # http://localhost:4321
pnpm build                                         # astro build + Pagefind index
pnpm test:smoke                                    # route smoke test (server must be running)
pnpm dlx wrangler deploy                           # deploy the Worker
```
