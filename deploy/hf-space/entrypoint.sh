#!/usr/bin/env bash
# Start the Cloudflare Tunnel (if configured) and the unified backend worker.
set -euo pipefail

DB_DIR="$(dirname "${SEC_DB_PATH:-/home/user/app/data/ex10_listener.db}")"
mkdir -p "$DB_DIR"

# Seed the DB on first boot so the API serves content immediately.
if [ ! -f "${SEC_DB_PATH}" ] && [ -f /home/user/app/seed.db ]; then
  echo "[entrypoint] seeding ${SEC_DB_PATH} from seed.db"
  cp /home/user/app/seed.db "${SEC_DB_PATH}"
fi

# Cloudflare Tunnel: connects this container's API to a Cloudflare hostname you
# configure (create the tunnel + route a hostname, then set TUNNEL_TOKEN as a
# Space secret). Without a token the API is still reachable via the HF Space URL.
if [ -n "${TUNNEL_TOKEN:-}" ]; then
  echo "[entrypoint] starting cloudflared tunnel"
  cloudflared tunnel --no-autoupdate run --token "${TUNNEL_TOKEN}" &
else
  echo "[entrypoint] TUNNEL_TOKEN not set — skipping cloudflared (API still on the HF Space URL)"
fi

echo "[entrypoint] starting worker (listener + backfill + API on :${SEC_API_PORT})"
exec python -m sec_listener.worker
