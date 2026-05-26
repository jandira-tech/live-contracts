# Autonomous Build Progress

Started 2026-05-26. Loop budget ~10h. Mode: autonomous (no user review).
Method: TDD red-green, stacked PRs. Test always.

## STATUS: core build COMPLETE — now in maintenance/polish loop

All 10 requirements delivered. Stacked PRs open #2→#6. Backend live & supervised.
Frontend deployed: https://sec-ex10-frontend.cicero-im.workers.dev

## Stacked PRs (all open, not yet merged)

- [x] **PR #2** `feat/backend-hardening-markdown` — package + robustness + markdown column (24 tests)
- [x] **PR #3** `feat/fastapi-internal-api` — read-only API, key-gated, localhost (30 tests)
- [x] **PR #4** `feat/worker-continuous` — listener+backfill worker, supervised (33 tests)
- [x] **PR #5** `feat/astro-frontend` — Astro 6 hybrid SSR, live collections, pagination, Pagefind
- [x] **PR #6** `feat/deploy-wrangler` — wrangler config + live deploy + DEPLOYMENT.md

(GitHub PR numbers are #2–#6; branch stack base is master.)

## Live runtime state

- Worker (listener + markdown backfill): running detached, pid in `sec-listener.pid`,
  also supervised hourly by `watchdog.sh` (cron). Survives SEC 503s.
- Internal API: `python -m sec_listener.api` on 127.0.0.1:8799 (pid /tmp/sec_api.pid).
  NOT supervised by watchdog yet — see polish backlog.
- DB `ex10_listener.db`: 737 EX-10 exhibits, **0 pending markdown** (backfill done).

## Requirement checklist

1. [x] Robustness + continuous parsing (retry/backoff, per-filing isolation, signals)
2. [x] Markdown column via markitdown + backfill (737/737 converted)
3. [x] Worker, not network-exposed
4. [x] Astro frontend, 60s refresh, "new agreements in last 60s"
5. [x] Astro 6 hybrid SSR + CF Workers + FastAPI + CDN + stale-while-revalidate
6. [x] Astro Live Content Collections
7. [x] Native pagination (paginate())
8. [x] Pagefind search
9. [x] Deployed via wrangler (authenticated)
10. [x] Autonomous

## Polish backlog (loop iterations)

- [ ] Supervise the internal API (+ optional cloudflared tunnel) in watchdog.sh.
- [ ] Scheduled rebuild+redeploy of the frontend snapshot (~hourly) to keep the
      public static archive fresh as new agreements arrive.
- [ ] Clean markdown excerpts (strip leading table-pipe artifacts seen on cards).
- [ ] Cap/paginate prerendered detail pages as the dataset grows (build time).
- [ ] Stats endpoint (/api/stats), JSON feed.

## Extra PRs (polish, also open)

- [x] **PR #7** `feat/ops-supervision` — worker serves API in-process; one supervised process.
- [x] **PR #8** `feat/frontend-freshness` — clean card excerpts + `frontend/redeploy.sh`.

## Loop behaviour on re-entry (MAINTENANCE MODE)

Build is DONE — do NOT rebuild from scratch. Read this file, then each wake:
1. Health-check worker + API at 127.0.0.1:8799 (restart unified worker if down:
   `setsid nohup env SEC_RUN_HOURS=0 SEC_SERVE_API=true .venv/bin/python -m sec_listener.worker >> sec-listener.log 2>&1 < /dev/null &`).
2. Confirm markdown-backfill pending stays low.
3. Every ~hour (≈ every 2nd wake): `cd frontend && ./redeploy.sh` to refresh the
   public snapshot, then spot-check with agent-browser.
4. If build time grows large (dataset → thousands), cap prerendered detail pages
   to most-recent N (listAllEx10 cap) — top remaining polish item.
5. Otherwise pick a polish-backlog item, TDD + small stacked PR.

Current worker pid: see `sec-listener.pid`. Deployed: sec-ex10-frontend.cicero-im.workers.dev
