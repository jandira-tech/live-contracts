# Live Contracts — Frontend (Astro 6, fully live SSR)

Astro 6 app on **Cloudflare Workers**, served at **[live-contracts.arthur.law](https://live-contracts.arthur.law)**.
Every page is **live SSR** — querying **Cloudflare D1** (Drizzle over the `DB` binding) at request
time, cached at the edge (CDN + stale-while-revalidate). **No rebuilds or redeploys** are needed
when content changes; deploy only for code.

## Architecture

```
Cloudflare D1 (authoritative, written by the backend via /api/ingest)
        ▲  Drizzle query via the `DB` binding (getDb(), cloudflare:workers env)
Cloudflare Worker (Astro SSR)  ──CDN / stale-while-revalidate──▶ users
```

Data access lives in `src/lib/api.ts` (Drizzle queries) + `src/db/` (schema + `getDb()` singleton).
The live loaders in `src/loaders/` call those functions, which default to `getDb()`.

- **Homepage** (`/`) — Live Content Collection: agreements in the last 60s, 60s auto-refresh.
- **Browse** (`/agreements/[page]`) — paginated, with a faceted sidebar (filing type, filer/CIK
  filters, newest/oldest sort). Ordering is by actual **filing time**.
- **Detail** (`/agreement/[id]`) — markdown → HTML; captured exhibit images; **Export to Cicero** pill.
- **Search** (`/search?q=`) — full-text search (LIKE over description + markdown).
- **Ingest** — `POST /api/ingest` (key-gated): the backend UPSERTs finalized rows into D1.

## Design system

Light editorial-technical theme (tokens in `src/styles/global.css`). Canonical brand fonts, both
**self-hosted** (latin-subset variable woff2 in `public/fonts/`, no external request): **Libre
Franklin** (body/display) and **Roboto Mono** (mono — labels, wordmark, code); Departure Mono is
retained as the mono fallback. The structural yellow accent is never used as text color. `bun run
test:fonts` guards the font wiring.

## Config

| Binding / Var | Meaning |
|-----|---------|
| `DB` (D1) | The `sec-ex10` D1 database, bound in `wrangler.jsonc`. Read via `getDb()`. |
| `SEC_API_KEY` | Secret gating `POST /api/ingest`. `wrangler secret put SEC_API_KEY` (same value the backend sends). |

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
