#!/usr/bin/env bash
# Read-only smoke test for Live Contracts.
# Usage: H=https://live-contracts.arthur.law ./smoke-prod.sh
# It only reads; the ingest check sends no key and expects a rejection.
# Needs: bash, curl, jq, GNU date. Exit status 1 when any check fails.
H=${H:-https://live-contracts.arthur.law}; fail=0
t(){ if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (want '$2', got '$3')"; fail=1; fi; }
rpc(){ curl -s -m 30 -X POST "$H/mcp" -H 'content-type: application/json' \
        -H 'accept: application/json, text/event-stream' -d "$1"; }

title=$(curl -s -m 20 "$H/" | grep -o -i '<title[^>]*>[^<]*' | head -1)
case "$title" in *"EX-10"*|*"Live Contracts"*) echo "ok   home title";; *) echo "FAIL home title: $title"; fail=1;; esac
t "robots.txt is ours"   1   "$(curl -s -m 20 "$H/robots.txt" | grep -c 'live-contracts.arthur.law/sitemap.xml')"
t "sitemap"              200 "$(curl -s -o /dev/null -w '%{http_code}' -m 20 "$H/sitemap.xml")"
t "search page"          200 "$(curl -s -o /dev/null -w '%{http_code}' -m 30 "$H/search")"
t "MCP server card"      200 "$(curl -s -o /dev/null -w '%{http_code}' -m 20 "$H/.well-known/mcp/server-card.json")"
t "markdown negotiation" "text/markdown; charset=utf-8" \
  "$(curl -s -o /dev/null -w '%{content_type}' -m 30 -H 'accept: text/markdown' "$H/")"
t "GET /mcp is 405"      405 "$(curl -s -o /dev/null -w '%{http_code}' -m 20 "$H/mcp")"
t "initialize" live-contracts "$(rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' | jq -r '.result.serverInfo.name')"
t "tools/list" "browse_agreements,get_agreement,list_recent_agreements,search_agreements" \
  "$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq -r '[.result.tools[].name]|sort|join(",")')"
filed=$(rpc '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_recent_agreements","arguments":{"limit":1}}}' \
        | jq -r '.result.content[0].text | fromjson | .items[0].filed_at' | cut -c1-8)
age=$(( ( $(date -u +%s) - $(date -u -d "${filed:-19700101}" +%s) ) / 86400 ))
if [ "$age" -le 4 ]; then echo "ok   freshness ($filed)"; else echo "FAIL freshness: newest filing $filed is $age days old"; fail=1; fi
t "ingest rejects no key" 401 "$(curl -s -o /dev/null -w '%{http_code}' -m 20 -X POST "$H/api/ingest" -H 'content-type: application/json' -d '{"rows":[]}')"
exit $fail
