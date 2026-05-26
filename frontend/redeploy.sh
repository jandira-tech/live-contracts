#!/bin/bash
# Rebuild the Astro snapshot (fresh agreements from the API) and redeploy the
# Cloudflare Worker. Safe to run on a schedule (e.g. hourly cron) to keep the
# prerendered archive + Pagefind index current. Requires the internal API
# reachable at SEC_API_URL (default http://127.0.0.1:8799).
set -euo pipefail
cd "$(dirname "$0")"

export SEC_API_URL="${SEC_API_URL:-http://127.0.0.1:8799}"

echo "[$(date -u +%FT%TZ)] redeploy: checking API at $SEC_API_URL"
if ! curl -sf "$SEC_API_URL/health" >/dev/null; then
  echo "[$(date -u +%FT%TZ)] redeploy ABORTED: API unreachable" >&2
  exit 1
fi

echo "[$(date -u +%FT%TZ)] building…"
./node_modules/.bin/astro build

echo "[$(date -u +%FT%TZ)] deploying…"
wrangler deploy

echo "[$(date -u +%FT%TZ)] redeploy complete"
