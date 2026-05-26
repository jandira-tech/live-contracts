#!/usr/bin/env bash
# Start the unified backend worker (SEC listener + markdown backfill + FastAPI).
# Hugging Face exposes the API publicly at the Space URL; it is key-gated via
# SEC_API_KEY. (Cloudflare Tunnel is intentionally not used — banned on HF Spaces.)
set -euo pipefail

DB_DIR="$(dirname "${SEC_DB_PATH:-/home/user/app/data/ex10_listener.db}")"
mkdir -p "$DB_DIR"

# Seed the DB on first boot so the API serves content immediately.
if [ ! -f "${SEC_DB_PATH}" ] && [ -f /home/user/app/seed.db ]; then
  echo "[entrypoint] seeding ${SEC_DB_PATH} from seed.db"
  cp /home/user/app/seed.db "${SEC_DB_PATH}"
fi

# Restore from the HF dataset (durable source of truth) when it's larger than the
# seed — so a fresh container catches up to accumulated history. No-op without HF_TOKEN.
echo "[entrypoint] restoring from HF dataset if newer"
python -m sec_listener.boot_restore || echo "[entrypoint] boot restore skipped/failed (continuing)"

echo "[entrypoint] starting worker (listener + backfill + API on :${SEC_API_PORT})"
exec python -m sec_listener.worker
