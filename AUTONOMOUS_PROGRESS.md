# Autonomous Build Progress

Started: 2026-05-26. Loop budget: ~10 hours. Mode: autonomous (no user review).
Method: TDD red-green, stacked PRs (each branch over the previous). Test always.

## Architecture (target)

- **Backend (Python, in-repo)**: hardened SEC RSS listener + EX-10 extraction, SQLite (`ex10_listener.db`).
  Adds `markdown` column (HTML→MD via `markitdown`, a solid dependency).
- **FastAPI API**: read-only, internal only (bound to localhost / API-key gated). NOT open to the world.
- **Worker**: background worker process driving continuous parsing + markdown backfill.
- **Frontend (Astro 6, hybrid SSR)**: Live Content Collections fetch from API at request time,
  refresh every 60s ("new agreements in the last 60 seconds"), native pagination, Pagefind search.
- **Edge**: Cloudflare Worker (Astro adapter) + CDN caching + stale-while-revalidate.
- **Deploy**: wrangler (authenticated, account 5651b2fe85f9bbb0fc1b8f4ad2cb4e64).

## Stacked PR plan

- [ ] **PR1** `feat/backend-hardening-markdown` (off master): refactor listener into testable package,
      robustness (retries/backoff, structured logging, graceful errors, config via env),
      add `markdown` column + markitdown converter, backfill. TDD.
- [ ] **PR2** `feat/fastapi-internal-api` (off PR1): read-only FastAPI — recent agreements, pagination,
      `/since?seconds=60`, API-key gated, localhost bind. TDD.
- [ ] **PR3** `feat/worker-continuous` (off PR2): background worker orchestrating listener + conversion;
      systemd/supervisor-style runner; not network-exposed. TDD.
- [ ] **PR4** `feat/astro-frontend` (off PR3): Astro 6 hybrid SSR, live collections loader, 60s refresh,
      pagination, Pagefind, Cloudflare adapter, SWR cache headers.
- [ ] **PR5** `feat/deploy-wrangler` (off PR4): wrangler config + deploy.

## Current state

- Iteration: 0 (setup)
- Active branch: (creating PR1)
- Next action: PR1 — scaffold backend package + first failing test for markdown converter.

## Notes / decisions

- Markdown dependency: `markitdown` (Microsoft) — handles HTML/PDF/txt, well maintained.
- DB stays at repo root `ex10_listener.db` (production listener + watchdog depend on it).
- Keep `sec-listener.py` working or update `watchdog.sh` to new entrypoint.
- FastAPI origin must not be public; Worker reaches it via shared secret / tunnel; document for user.
