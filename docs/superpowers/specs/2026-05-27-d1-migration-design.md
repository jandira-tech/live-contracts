# Design: Migrate the content store to Cloudflare D1

**Date:** 2026-05-27
**Status:** Approved (design); implementation pending
**Topic:** Replace the ephemeral SQLite-on-HF-Space + HF-parquet-as-DB content store with
Cloudflare **D1** as the durable, authoritative store, read directly by the Astro Worker.

## Problem

Today the content store is SQLite living on a disk-less HF Space, made durable by mirroring the
whole table to a single HF parquet (`data/exhibits.parquet`) every ~15 min and restoring it on
every cold boot. Three things blow up as the dataset grows:

1. **Full-rewrite sync** — the entire parquet (markdown included) is rewritten and re-uploaded each
   cycle → O(everything) bandwidth + unbounded HF LFS/commit-history bloat.
2. **Boot-restore** — every cold start downloads and loads *all* rows into SQLite.
3. **No incremental write** — empirically confirmed: a HF dataset is parquet files in a git repo,
   not a SQL backend. DuckDB can `SELECT` over `hf://` but `INSERT`/`COPY TO hf://` both fail
   (`NotImplementedException: Writing to HTTP files not implemented`). Mutation = a git commit of a
   file, which is why the code rewrites the whole file.

**D1 is a real, durable SQLite** that supports incremental `INSERT`/`UPSERT`, so all three problems
disappear. The Astro Worker can also read it directly at the edge instead of round-tripping to the
HF Space.

## Decisions (locked)

- **Full markdown lives in D1**, on **Workers Paid** (10 GB/db cap; ~67 KB markdown/row → ~3.3 GB at
  50k rows, comfortable. The free 500 MB cap would fill at ~7.5k rows, so Workers Paid is a prerequisite).
- **Images stay on HF** unchanged (blobs under `images/<accession>/`, referenced by the `image_urls`
  column). No bytes-in-D1.
- **HF content table is kept as a public export from D1** (phase 2): periodically export D1 → parquet
  → `arthrod/sec-ex10-exhibits`, preserving the public/streamable dataset. This *replaces* the old
  SQLite→parquet mirror as a downstream artifact, not an operational dependency.

### Stack decisions (validated against emdash-cms)

Reviewed `emdash-cms/emdash` (an Astro-on-Cloudflare CMS) `packages/core/src/db` + `packages/cloudflare/src/db/d1.ts`:

- **Binding access:** `import { env } from "cloudflare:workers"` gives the D1 binding at *module scope*
  anywhere in the Worker (pages, `lib/`, **and live loaders**). This is how emdash's D1 adapter reads
  its binding. So the binding is *not* request-scoped-only — we use a module-level singleton `getDb()`
  and do **not** thread `Astro.locals.runtime.env.DB` through functions. The earlier worry ("live
  loaders can't see the binding") is moot; **the live-collection loaders stay**, calling `getDb()`.
- **Query layer: Drizzle ORM** (`drizzle-orm/d1`) with a TS schema and `drizzle-kit`-generated
  migrations applied via `wrangler d1 migrations apply`. Type-safe, composable, and portable (D1 ↔
  Turso/libSQL ↔ Postgres) if we ever move stores.
- **No D1 Sessions API / request-scoped middleware.** emdash uses `createRequestScopedDb` + bookmark
  cookies only for read-replica read-your-writes. Our writes come from a *separate origin* (HF worker)
  and reads are CDN-cached, so a singleton handle (`session: disabled`) suffices. Sessions remain a
  future perf/consistency lever, not needed now.
- **Testability:** data functions take an optional `db: DB = getDb()`; tests pass a Drizzle instance
  built over `cloudflare:test`'s `env.DB`, production calls use the singleton.

## Target architecture

```
SEC ──> [HF Space Python worker]            (ingestion only — datamule/markitdown stay in Python)
            │ enrich in local SQLite: pending → markdown → filing_metadata → image_urls
            │ when a row FINALIZES, POST it once (idempotent) →
            ▼
        [Astro Worker]  POST /api/ingest  (auth: SEC_API_KEY)  ──UPSERT──▶  D1 (authoritative, durable)
            ▲                                                                 │
            └────────── reads feed / search / facets / detail / stats via env.DB binding ◀──┘
        CDN / SWR ──▶ live-contracts.arthur.law

   (phase 2)  D1 ──export──▶ data/exhibits.parquet on HF  (public dataset; images already there)
```

### Components

1. **D1 database** (`sec-ex10` / `live-contracts`), bound to the Astro Worker as `env.DB`.
   - Table `exhibits`: same columns as today — `id` (PK), `accession`, `cik`, `form_type`,
     `doc_type`, `filename`, `description`, `sequence`, `filing_url`, `found_at`, `markdown_status`,
     `filing_metadata` (JSON text), `image_urls` (JSON text), `markdown` (TEXT).
   - `filed_at` stored as an explicit column (extracted from `filing_metadata.filed_at` at ingest)
     so it can be indexed without relying on a generated-column + `json_extract` (verify D1 support;
     explicit column is the safe path).
   - Indexes: `filed_at DESC`, `form_type`, `cik`. Unique key on `(accession, doc_type, filename)`
     — matching the source SQLite `ex10_exhibits` constraint exactly — so ingest is idempotent and
     distinct exhibits aren't coalesced.
   - Search: SQLite **FTS5** virtual table over `description`+`markdown` if D1 supports it (verify in
     PR2); otherwise `LIKE` fallback (current behavior).

2. **Astro Worker — read layer.** Replace the `fetch`-the-FastAPI calls in `frontend/src/lib/api.ts`
   with D1 queries via `Astro.locals.runtime.env.DB`. Covers: feed/archive (`/agreements/[page]`),
   facets, search, detail (`/agreement/[id]`), stats, and the live homepage `since` query. Ordering
   is `filed_at DESC NULLS LAST, found_at DESC, id DESC` (unchanged semantics). The FastAPI **read**
   API retires for the frontend.

3. **Astro Worker — ingest route** `POST /api/ingest` — *the backend→Cloudflare connection.*
   - This is a **new write path**: today data flows Worker→HF (reads); D1 reverses it, so the HF
     Space must reach Cloudflare to land rows. It does so as a plain outbound HTTPS POST to this route
     (HF Spaces already make outbound HTTPS to SEC, so no new egress/network setup).
   - Auth: `X-API-Key == SEC_API_KEY` (Worker secret) → 401 otherwise. **The credential already
     exists on both sides** — the Worker holds `SEC_API_KEY` as a secret and the HF Space already has
     the same value; the writer just sends the key it already has. No Cloudflare account API token
     anywhere; D1 access never leaves Cloudflare (binding only).
   - Body: JSON batch of finalized rows (chunked to stay under the Worker request-size/CPU ceiling).
   - `env.DB.batch([...])` of `INSERT ... ON CONFLICT(accession,filename) DO UPDATE` (idempotent).
   - Returns the accepted accessions so the writer can mark them mirrored.
   - Route lives on the main Worker (`live-contracts.arthur.law/api/ingest`). Variant (not chosen): a
     separate `*.workers.dev` ingest Worker bound to the same D1 to keep writes off the public domain.

4. **HF Space Python worker — push finalized rows** (`sec_listener/d1_sync.py`).
   - SQLite gains a `mirrored INTEGER DEFAULT 0` column (`_ensure_column`).
   - **Finalized predicate:** `markdown_status` terminal (`done`/`empty`/`error`) AND
     `filing_metadata IS NOT NULL` AND `image_urls IS NOT NULL` AND `mirrored = 0`.
   - Loop (replaces `_hf_sync_loop`): batch finalized rows → POST to `/api/ingest` → on success set
     `mirrored = 1`. Network failure leaves `mirrored = 0` → retried; never crashes ingestion.
   - Config: `D1_INGEST_URL` (default `https://live-contracts.arthur.law/api/ingest`) + the existing
     `SEC_API_KEY` (already an HF Space secret). No new credential.
   - **Image retry cap:** image capture currently leaves `image_urls = NULL` and retries forever; a
     row that never captures would never finalize. Add a bounded retry (e.g. `image_attempts`
     column; after N tries set `image_urls = []`) so every row finalizes eventually.

5. **Boot-restore retirement.** D1 is durable, so the HF-parquet-as-DB restore is dropped. The HF
   Space SQLite becomes a *working/staging buffer* only. On restart the worker starts fresh;
   re-discovered rows re-ingest harmlessly because `/api/ingest` is an idempotent UPSERT. Optional
   optimization: seed the in-session "seen" set from D1 (a lightweight query) to skip re-converting
   already-finalized rows.

6. **HF content export (phase 2).** A scheduled job (Cloudflare Cron Trigger Worker, or a Python
   job) reads D1 → writes parquet → uploads `data/exhibits.parquet`. Replaces the old mirror's role.

### Data flow

- **Write:** SEC → worker SQLite (enrich) → finalize → `POST /api/ingest` → D1 UPSERT.
- **Read:** user → Astro SSR → D1 query → render → CDN/SWR cache.
- **Public export (phase 2):** D1 → parquet → HF.

### Error handling

- Ingest: invalid key → 401; malformed body → 400; D1 batch failure → return accepted subset so the
  writer retries the rest. UPSERT keeps retries safe.
- Writer push: failures leave `mirrored = 0`; retried next loop. SQLite is the durable staging buffer
  so no row is lost if D1 is briefly unavailable.
- Read layer: D1 query failure degrades gracefully (existing "temporarily unavailable" behavior).

## Migration of existing data

Reuse the ingest path: a one-off script reads the current HF parquet (934 rows) and POSTs all rows
to `/api/ingest` in batches. This migrates data *and* exercises the ingest endpoint. Verify
`SELECT COUNT(*)` in D1 == source count before cutover.

## Testing (TDD, stacked PRs)

- **Backend (`pytest`):** `d1_sync` finalized-row selection, batching, mark-mirrored-on-success,
  retry-on-failure (fake HTTP poster); image-retry-cap → finalization.
- **Worker (`@cloudflare/vitest-pool-workers`, real local D1):** read queries (ordering, facets,
  pagination, search) and the ingest route (auth, UPSERT idempotency). New frontend devDependency.
- **Migration:** dry-run + row-count parity check.

### PR stack (each over the previous)

1. **PR1** — create D1 + schema + `wrangler.jsonc` binding; migration script; load the 934 rows.
   (D1 exists with data; nothing reads it yet.)
2. **PR2** — frontend read layer → D1 (retire FastAPI fetches); vitest-pool-workers tests.
3. **PR3** — `POST /api/ingest` route (auth + UPSERT) + tests.
4. **PR4** — Python worker `d1_sync` push loop + finalized predicate + image retry cap; retire
   boot-restore / parquet-as-DB. Tests.
5. **PR5 (phase 2)** — D1 → HF parquet public export (scheduled).

## Prerequisites / verify during implementation

- **Workers Paid** — ✅ confirmed active (D1 cap is 10 GB).
- **Backend→Cloudflare connection** — ✅ decided: ingest route on the main Worker, authed with the
  shared `SEC_API_KEY` (no new Cloudflare token).
- D1 **FTS5** support — confirm in PR2; `LIKE` fallback if absent.
- `@cloudflare/vitest-pool-workers` for Worker tests with a real local D1.
- Astro + Cloudflare adapter exposes the D1 binding at `Astro.locals.runtime.env.DB`.

## Out of scope

- Moving images off HF (explicitly kept on HF).
- Replacing the FastAPI *health* surface (the HF Space still runs the ingestion worker).
- Any frontend visual/redesign change.
