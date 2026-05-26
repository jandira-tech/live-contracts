#!/usr/bin/env bash
# Stage the HF Space build context: copy the backend package and regenerate a
# lean seed DB snapshot from the live ex10_listener.db. Run from repo root.
# Then push with:  hf upload arthrod/sec-ex10-backend deploy/hf-space . --repo-type space
set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root

DEST=deploy/hf-space
rm -rf "$DEST/sec_listener"
cp -r backend/sec_listener "$DEST/sec_listener"
find "$DEST/sec_listener" -name __pycache__ -type d -exec rm -rf {} + 2>/dev/null || true

# Lean, consistent seed: ex10_exhibits (+markdown) + seen_accessions, VACUUMed.
python - <<'PY'
import sqlite3
src = sqlite3.connect("ex10_listener.db")
dst = sqlite3.connect("deploy/hf-space/seed.db")
src.backup(dst); src.close()
dst.execute("DROP TABLE IF EXISTS all_exhibits")
dst.execute("DROP TABLE IF EXISTS rss_entries")
dst.commit(); dst.execute("VACUUM")
print("seed ex10_exhibits:", dst.execute("SELECT COUNT(*) FROM ex10_exhibits").fetchone()[0])
dst.close()
PY
echo "staged $DEST (sec_listener/ + seed.db)"
