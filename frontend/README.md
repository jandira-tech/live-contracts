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

- **Homepage** (`/`) — Live Content Collection: the 100 newest agreements. A small inline script
  re-fetches the page every 60s and inserts only the new cards (scroll position survives); without
  JS a `<noscript>` meta refresh does the same the old way.
- **Browse** (`/agreements/[page]`) — paginated, with a faceted sidebar (filing type, filer/CIK
  filters, newest/oldest sort). Ordering is by actual **filing time**.
- **Detail** (`/agreement/[id]`) — markdown → HTML with relative image refs resolved against the EDGAR
  filing folder (image-only exhibits); captured exhibit images; sticky filing aside with Export to
  Cicero, the SEC filing and a Markdown view.
- **Search** (`/search?q=`) — FTS5 full-text search (`exhibits_fts`, migration 0003) with hit snippets;
  every token is quoted so FTS operators in user input are literal. The old `LIKE '%q%'` scan over
  19k markdown bodies is kept only as a fallback for a database without the index.
- **Ingest** — `POST /api/ingest` (key-gated): the backend UPSERTs finalized rows into D1.

## Design system

The page follows the Arthur Souza Rodrigues design system (the same one as arthur.law and
prompting-for-attorneys): warm paper `--bg`, one ink `--ink`, hairlines `--faint`, mono uppercase
labels prefixed with `/`, the `--mark` highlighter used as a background swipe, badge or arrow chip
and never as a resting button fill, square corners, and a single hard offset shadow on hover.
Tokens live in `src/styles/global.css` under the design-system names; the old names (`--text`,
`--accent`, `--border`, `--max-width`) resolve as aliases. No mono label is set below 12px, and
there are no emoji or icon glyphs (the system is type-only).

The header is the shared navbar: the `ARTHUR.SOUZA.RODRIGUES` wordmark links to https://arthur.law/
and the `/ Live contracts` sub-label links to this site's root. Under 760px the sub-label and nav
links hide and a fixed bottom bar carries Live / Browse / Search. The A / A+ toggle persists a
text-size preference in `localStorage` (`asr-size`).

Fonts are self-hosted (latin-subset variable woff2 in `public/fonts/`, no external request):
**Libre Franklin** (body/display) and **Roboto Mono** (labels, wordmark, code); Departure Mono is
the mono fallback. `bun run test:fonts` guards the font wiring and `bun run test:design` guards
the navbar, tokens, type floors and the no-emoji rule.

## Card headlines

94 of 100 live rows carry a label-only SEC `description` (`EX-10.1`, `EXHIBIT 10.34`). Cards and the
detail page therefore derive their headline with `displayTitle()` (`src/lib/summary.ts`): a short
standalone line naming the instrument, a letter's `Re:` line, or the ALL-CAPS opener of the first
sentence, title-cased; the label stays as the fallback. The whole card is one click target (a
stretched link on the title), and cards without text show one of three labelled empty states:
scanned pages captured, image-only on EDGAR, or text pending.

## Config

| Binding / Var | Meaning |
|-----|---------|
| `DB` (D1) | The `sec-ex10` D1 database, bound in `wrangler.jsonc`. Read via `getDb()`. |
| `SEC_API_KEY` | Secret gating `POST /api/ingest`. `wrangler secret put SEC_API_KEY` (same value the backend sends). |

## Develop / deploy

```bash
bun install
bun run db:setup         # migrate + seed the LOCAL D1 that astro dev reads (.wrangler/state)
bun run dev              # http://localhost:4321
bun run build            # fast — only /404 is static; everything else is SSR
bun run check            # astro check (0 errors expected)
bun run test:unit        # vitest on the Workers pool with a local D1
bun run test:fonts       # font-loading guard (no server needed)
bun run test:design      # design-system conformance guard (no server needed)
bun run test:smoke       # BASE=<url> node test/smoke.mjs
bun run db:migrate       # wrangler d1 migrations apply sec-ex10 --remote (run BEFORE deploying 0003)
bun run db:fts-backfill -- --remote   # one-off, resumable: index rows that predate the FTS triggers
wrangler deploy          # deploy code changes (content needs no redeploy)
```

Live: **https://live-contracts.arthur.law**
