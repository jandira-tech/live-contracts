# live-contracts

[![license](https://badgen.net/badge/license/Apache-2.0/blue)](./LICENSE)
[![frontend](https://badgen.net/badge/frontend/Astro%206%20%C2%B7%20Cloudflare%20Workers/orange)](./frontend)
[![backend](https://badgen.net/badge/backend/FastAPI%20%C2%B7%20SQLite/green)](./backend)
[![site](https://badgen.net/badge/live/live-contracts.arthur.law/blue)](https://live-contracts.arthur.law)
[![github](https://badgen.net/badge/icon/jandira-tech%2Flive-contracts?icon=github&label)](https://github.com/jandira-tech/live-contracts)

A near-real-time feed of **EX-10 (material contract) exhibits** as they are filed with the SEC.
A hardened listener watches EDGAR, extracts each EX-10 exhibit, converts it to Markdown, and serves
it through a read-only API; an Astro frontend on Cloudflare Workers renders it live at the edge.

- Live: **[live-contracts.arthur.law](https://live-contracts.arthur.law)**
- Repo: [jandira-tech/live-contracts](https://github.com/jandira-tech/live-contracts)
- Maintainer: [jandira.tech](https://www.jandira.tech) — We are building legal tech. Jandira Technologies
  is the studio behind tools like [Cicero](https://arthur.law) (a legal workbench that turns messy inputs
  into redlines, issue lists, and memos), PII redaction models for Brazilian Portuguese, and AI/contract
  benchmarks. `live-contracts` falls out of that work — material-contract exhibits are where deal terms
  actually live, so watching them stream off EDGAR in real time is its own small piece of infrastructure.

> Public SEC filing data only. Not legal or investment advice. Not affiliated with the SEC.

## Architecture

Two halves. The **backend** is an internal ingestion worker — it talks to SEC and its own working
SQLite only. **Cloudflare D1** is the durable, authoritative store; the **Astro Worker** is the only
public surface and reads D1 directly (Drizzle over the `DB` binding), edge-cached, so content updates
need **no rebuild or redeploy** — only code changes do.

```
SEC EDGAR
   │  poll (≤10 rps, backoff)
   ▼
[HF Space worker] listener + markdown / filing-header / image backfill → working SQLite
   │  when a row is FINALIZED, POST it once (idempotent, X-API-Key)
   ▼
[Astro Worker] POST /api/ingest ──UPSERT──▶  Cloudflare D1 (authoritative)
   ▲                                              │
   └────────── reads feed / search / facets / detail via Drizzle binding ◀──┘
   CDN / stale-while-revalidate ──▶  live-contracts.arthur.law
```

Images stay on Hugging Face as blobs+URLs. The HF dataset `arthrod/sec-ex10-exhibits` is a phase-2
**public export** from D1 (DuckDB-queryable parquet), not an operational dependency.

## Layout

```
backend/      ingestion worker (listener + backfill); pushes finalized rows to D1 — see backend/README.md
frontend/     Astro 6 SSR on Cloudflare Workers, reads D1 via Drizzle — see frontend/README.md
deploy/
  hf-space/   Docker Space bundle (the backend worker, deployed to Hugging Face)
DEPLOYMENT.md  end-to-end deploy guide (backend worker + D1 + Worker)
```

## API (read-only, `X-API-Key`)

| Endpoint | Purpose |
|----------|---------|
| `GET /health` | liveness (open, no key) |
| `GET /api/ex10?page=&page_size=&form=&cik=&filer=&sort=` | paginated feed, filterable + sortable (newest/oldest by filing time) |
| `GET /api/ex10/since?seconds=` | exhibits seen in the last N seconds (live feed) |
| `GET /api/ex10/{id}` | one exhibit + Markdown + parsed image URLs |
| `GET /api/facets` | filing-type counts for faceted browse |
| `GET /api/search?q=` | full-text search over description + Markdown |
| `GET /api/stats` | totals strip |

## Develop

```bash
# backend
cd backend && uv pip install -e . && PYTHONPATH=. python -m pytest tests/
# frontend
cd frontend && bun install && bun run dev      # http://localhost:4321
bun run test:fonts                              # font-loading guard
```

Deploy: see [DEPLOYMENT.md](./DEPLOYMENT.md).

## Find us

[arthur.law](https://arthur.law) · [LinkedIn](https://linkedin.com/in/arthrod) · [Hugging Face](https://huggingface.co/arthrod) · `contact@arthur.law`
