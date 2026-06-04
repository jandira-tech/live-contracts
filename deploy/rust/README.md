# Deploy: Rust SEC EX-10 backend

A single-container **producer**: it watches SEC EDGAR (RSS `getcurrent` + EFTS,
mimicking datamule), extracts EX-10 exhibits → Markdown, and POSTs finalized rows
to the D1 ingest route. Validated at **100% EX-10 parity** with the Python
listener (see `../../COMPARISON_REPORT.md`).

## Deploy

```bash
cd deploy/rust
cp .env.example .env
# Edit .env — at minimum set SEC_API_KEY (and HF_TOKEN if you want image capture).
docker compose up -d --build
docker compose logs -f          # watch it discover + ingest
```

### Generate the `SEC_API_KEY` secret

```bash
openssl rand -hex 32                                   # 64-char hex secret
# or: python3 -c "import secrets; print(secrets.token_urlsafe(32))"
```

`SEC_API_KEY` is a **shared** secret: the same value must be configured on the
ingest route (`live-contracts.arthur.law/api/ingest`). Generate a fresh one only
when you control both sides — if you're pointing at an ingest route that already
has a key, use **that** existing value instead. To set both at once:

```bash
KEY=$(openssl rand -hex 32)
# Replace the empty placeholder in-place — don't append with >>, that leaves a
# duplicate SEC_API_KEY= line and some parsers pick the empty one.
sed -i "s|^SEC_API_KEY=.*|SEC_API_KEY=$KEY|" .env
# …and set the same $KEY as the ingest route's secret (e.g. wrangler secret put).
```

Verify it's alive:

```bash
curl -s http://localhost:7860/health      # {"status":"ok","total_seen":N}
```

`total_seen` is the number of EX-10-bearing filings processed since start.

## Required keys

| Key | Required | Purpose |
|-----|----------|---------|
| `SEC_API_KEY` | **yes** | `X-API-Key` for the ingest route — POSTs are rejected without the matching key |
| `D1_INGEST_URL` | yes (has prod default) | where finalized rows are POSTed |
| `HF_TOKEN` | optional | enables scanned-exhibit image capture → HF dataset; blank = skip |
| `SEC_USE_RSS` / `SEC_USE_EFTS` | optional (default `true`) | discovery sources; keep both on |
| `SEC_POLL_INTERVAL_MS`, `SEC_PUSH_BATCH`, `SEC_CONVERT_MARKDOWN`, `PORT`, `RUST_LOG`, `SEC_IMAGE_REPO` | optional | tuning (defaults in `.env.example`) |

## Cloudflare tunnel

You already run a tunnel, so there's nothing to add to the compose — it just
publishes `127.0.0.1:7860` and your tunnel connects into it. The only inbound
surface is `GET /health`, so this is only needed if you want health reachable
externally (the producer otherwise makes purely outbound calls).

Add one ingress rule to your existing tunnel config:

```yaml
ingress:
  - hostname: sec-ex10.example.com
    service: http://localhost:7860
  - service: http_status:404
```

(If your tunnel runs as a *container* rather than on the host, `localhost`
won't reach the app container — drop the `ports:` publish and put both on a
shared Docker network, then route the tunnel at `http://sec-ex10-rust:7860`.)

## Notes

- The runtime image is minimal (no shell tooling), so there's no in-container
  healthcheck by default — health is observed via the tunnel / external monitor
  hitting `/health`. To add one, install `curl` in the runtime stage of
  `backend-rust/Dockerfile` and uncomment the `healthcheck` block in the compose.
- **Before pointing at production `D1_INGEST_URL`:** the Rust backend writes
  `filing_url` in the accession-subfolder form
  (`.../data/{cik}/{accession_no_dashes}/{accession_dashed}.txt`) whereas the
  Python/HF-Space producer writes the flat legacy form
  (`.../data/{cik}/{accession_dashed}.txt`). Both resolve (HTTP 200) but differ
  as strings — decide whether to reconcile the format so the same D1 table stays
  consistent across producers.
