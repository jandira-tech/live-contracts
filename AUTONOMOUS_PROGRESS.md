# Autonomous Build Progress

Started 2026-05-26. Autonomous (no user review). TDD + stacked PRs.

## CURRENT ARCHITECTURE (fully live, zero content redeploys)

```
HF Space  arthrod/sec-ex10-api  (Docker: unified worker = SEC listener + markdown
  backfill + FastAPI on :7860, key-gated)  →  https://arthrod-sec-ex10-api.hf.space
        ▲ request-time fetch (SEC_API_URL + SEC_API_KEY)
Cloudflare Worker (Astro 6 SSR)  →  https://sec-ex10-frontend.cicero-im.workers.dev
        CDN + stale-while-revalidate caching
```

- Every frontend page is **live SSR** (homepage live collection 60s feed; archive,
  detail via getLiveEntry, search via /api/search). **No rebuilds for content.**
- Backend runs entirely on the **HF Space** (self-contained worker). The local
  host worker (127.0.0.1:8799) is now LEGACY/redundant (kept running, harmless;
  cron watchdog still restarts it). Production data source = the HF Space.
- API key: `62e44d3b3ce10702f7bc7ca9e9056a727e2bd29de2b696d1` (also /tmp/sec_api_key.txt),
  set as HF Space secret `SEC_API_KEY` and CF Worker secret `SEC_API_KEY`.

## Why no Cloudflare Tunnel
HF's abuse-handler BANS `cloudflared` on Spaces (it flagged our first Space). HF
already exposes a public HTTPS endpoint, so the tunnel is unnecessary. Backend is
key-gated; CF caches it at the edge.

## PRs (stacked on master)
#2 backend hardening+markdown · #3 internal API · #4 worker · #5 astro (prerender)
· #6 deploy · #7 ops-supervision · #8 excerpt+freshness · #9 stats ·
**#10 HF Space backend + live /api/search** · **#11 fully-live frontend (drops Pagefind+redeploys)**

## Requirement checklist (all met, re-architected per user)
1✅ robustness/continuous parsing  2✅ markdown (markitdown)  3✅ worker
4✅ live 60s feed  5✅ Astro6 SSR + CF Workers + FastAPI + CDN/SWR (FastAPI now on HF)
6✅ live collections  7✅ pagination (live SSR)  8↔ search now live /api/search
(Pagefind dropped per "fully live" choice)  9✅ wrangler deploy  10✅ autonomous

## Verified 2026-05-26 ~15:41Z
Backend refreshing confirmed: HF Space captured a new EX-10 live (801→802,
"exhibit101facilityagreem.htm"); ~50-60 filings processed per ~2.5-min poll cycle.
EX-10s are sparse and MORNING-heavy (a few/day, mostly AM ET) — daytime gaps are
NORMAL, not a fault. Poll interval 60s (cycle ~2min: paginates ~33 feed pages).
Review fixes shipped: search wildcard-escaping + SUBSTR(markdown) truncation,
8s fetch timeout, NYC time display, build-context CI guard. PR #12.

## Loop behaviour on re-entry (MAINTENANCE MODE — 30min cadence)
Build DONE. Do NOT rebuild/redeploy for content (site is fully live). Each wake:
1. Health-check HF Space: `curl -s https://arthrod-sec-ex10-api.hf.space/health`
   (expect {"status":"ok",...}). If PAUSED/SLEEPING, the HF free tier sleeps after
   inactivity — a request wakes it; check `hf spaces info arthrod/sec-ex10-api`.
2. Health-check deployed site: `curl -s https://sec-ex10-frontend.cicero-im.workers.dev/health-ish`
   i.e. fetch `/agreements/1` and confirm cards present.
3. NO redeploys needed. Only `wrangler deploy` (frontend) or `hf upload ... deploy/hf-space`
   (backend) for CODE changes.
4. Optional: keep the HF Space awake by pinging /health; refresh the HF seed via
   deploy/hf-space/build-context.sh + re-upload only if you want history persisted.
5. Pick polish items only if clearly valuable; otherwise just confirm health.

Local legacy worker pid: `sec-listener.pid`. Deployed frontend: cicero-im.workers.dev.
