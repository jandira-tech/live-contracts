#!/usr/bin/env bash
# DNS for AI Discovery (DNS-AID) records for live-contracts.arthur.law.
#
# Publishes ServiceMode SVCB records under the `_agents` namespace so agents can
# discover the site's entrypoint + MCP server via DNS (draft-mozleywilliams-dnsop-dnsaid).
#
# Requires a Cloudflare API token with **Zone → DNS → Edit** on the arthur.law zone
# (wrangler's OAuth token does NOT have this scope). Create one at:
#   https://dash.cloudflare.com/profile/api-tokens  (template: "Edit zone DNS")
#
# Usage:
#   export CF_API_TOKEN="<token>"
#   bash scripts/setup-dns-aid.sh
#
# DNSSEC: the spec mandates the discovery zone be DNSSEC-signed. Enable it once in
# the dashboard (arthur.law → DNS → Settings → DNSSEC → Enable); it is NOT done here.
set -euo pipefail

: "${CF_API_TOKEN:?set CF_API_TOKEN to a token with Zone:DNS:Edit on arthur.law}"
ZONE_NAME="arthur.law"
HOST="live-contracts.arthur.law"
API="https://api.cloudflare.com/client/v4"

zone_id=$(curl -fsS -H "Authorization: Bearer ${CF_API_TOKEN}" \
  "${API}/zones?name=${ZONE_NAME}" | python3 -c "import sys,json;print(json.load(sys.stdin)['result'][0]['id'])")
echo "zone ${ZONE_NAME} → ${zone_id}"

# ServiceMode (priority 1) SVCB records. alpn=h2 over 443; mandatory marks the
# required SvcParams. `_index` = discovery entrypoint, `_mcp` = the MCP service.
publish() {
  local label="$1"
  local name="${label}._agents.${HOST}"
  echo "→ ${name}"
  curl -fsS -X POST "${API}/zones/${zone_id}/dns_records" \
    -H "Authorization: Bearer ${CF_API_TOKEN}" -H "Content-Type: application/json" \
    -d "{\"type\":\"SVCB\",\"name\":\"${name}\",\"ttl\":3600,\"data\":{\"priority\":1,\"target\":\"${HOST}\",\"value\":\"alpn=\\\"h2\\\" port=443 mandatory=alpn,port\"}}" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print('  ', 'created' if d['success'] else d['errors'])"
}

publish _index
publish _mcp

echo "Done. Verify:  dig SVCB _index._agents.${HOST} @1.1.1.1 +short"
