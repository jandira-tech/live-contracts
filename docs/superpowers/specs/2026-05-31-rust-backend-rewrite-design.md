# Rust backend rewrite + parallel deploy stack — Design

**Date:** 2026-05-31
**Branch:** `feat/rust-backend-rewrite`
**Status:** Approved design, pre-implementation

## 1. Goal

Replace the Python SEC EX-10 listener backend with a Rust rewrite built on the
[`secinfra`](https://github.com/arthrod/secinfra-rust) crate, and stand up a
**parallel** deployment stack — a new Hugging Face Docker Space (Rust backend)
and a new Cloudflare Worker (Astro frontend) reading a **fresh, empty D1** —
alongside the existing live stack. No DNS cutover and no changes to the live
deployment in this work; cutover is a deliberate follow-up.

Decisions captured during brainstorming:

- **Full Rust backend rewrite** (not a hybrid listener-only swap).
- **Streamlined pipeline, no SQLite buffer.** Monitor → per-submission
  download/parse/filter/convert/enrich → POST once to `/api/ingest`. D1's
  idempotent upsert is the source of truth.
- **Images included in v1** (inline per-submission, since there is no backfill loop).
- **Parallel now, cut over later.** Keep `arthrod/sec-*` naming, new instances.
- **Fresh empty D1** for the new stack.
- **No new HF dataset.** "conejo-code" was a stray skill reference, dropped from scope.
- Rust crate lives in **`backend-rust/`** in this repo; **`secinfra` source is
  vendored in** (copied under `backend-rust/vendor/secinfra/`, including its C
  SGML `vendor/secsgmlc/` + `build.rs`) — no external git dependency at build time.
- Markdown via **`htmd`** (HTML-only). PDF/DOCX exhibits are a known delta
  (→ `empty` status); the image path catches scanned ones.

## 2. Topology

```
SEC EDGAR
   │  secinfra::Monitor (RSS + EFTS), rate-limited
   ▼
[NEW HF Space: arthrod/sec-ex10-api-rust]  ── Rust Docker, axum GET /health on :7860
   │  per submission: download SGML (.txt) → ParsedSgml → filter EX-10 →
   │  htmd/pdf markdown + filing header + (image-only?) HF graphic upload
   │  POST { rows:[…] } once, header X-API-Key   (bounded concurrency + retry queue)
   ▼
[NEW Worker: sec-ex10-frontend-v2]  POST /api/ingest ──UPSERT──▶  NEW D1 "sec-ex10-v2" (empty)
   ▲                                                                    │
   └────────── Astro reads feed / search / detail via Drizzle `DB` binding ◀┘
              (frontend code reused verbatim; only wrangler config differs)
```

The **D1 schema, the `/api/ingest` Astro route, and the ingest JSON contract are
unchanged.** The Rust producer matches the existing wire format exactly. The
Astro frontend source is reused verbatim — only deploy config changes.

## 3. The contract the Rust backend must satisfy

### 3.1 EX-10 classification (exact parity with `parsing.classify_documents`)

For each parsed SGML document with a `doc_type`:

- `doc_type` (trimmed) starts with `EX-10`:
  - suffix = `doc_type[5:]`. If suffix is empty **or** starts with `.` →
    **traditional EX-10** (material contract) → ingest.
  - otherwise (e.g. `EX-101`, `EX-100` XBRL) → "other EX-", **not** ingested.
- starts with `EX-` but not `EX-10` → other EX-, not ingested.
- anything else → ignored.

Only traditional EX-10 docs become rows. (The Python code also recorded "other"
exhibits in a local `all_exhibits` table; that table is **not** part of the D1
contract and is dropped — it was never pushed.)

### 3.2 Ingest record (exact JSON keys, from `d1_sync._FIELDS` + `ingest.ts InRow`)

POST body: `{ "rows": [ <record>, … ] }`, header `X-API-Key: <SEC_API_KEY>`.
Each record:

```json
{
  "id": <number>,            // echo token only (see 3.3); D1 mints the real UUIDv7
  "accession": "<dashed>",   // e.g. 0001234567-25-000123
  "cik": "<digits>",
  "form_type": "<RSS category term>",
  "doc_type": "EX-10.1",
  "filename": "ex10-1.htm",
  "description": "<from SGML doc>",
  "sequence": "<from SGML doc>",
  "filing_url": "https://www.sec.gov/Archives/edgar/data/<cik>/<dashed>.txt",
  "found_at": "<ISO-8601 detected time>",
  "filed_at": "<acceptance-datetime YYYYMMDDHHMMSS or ''>",
  "markdown_status": "done|empty|error",
  "filing_metadata": "<JSON string of filing header, see 3.5>",
  "image_urls": "<JSON string array of HF URLs, or omitted/null>",
  "markdown": "<converted text or ''>"
}
```

Constraints (enforced by `ingest.ts`): **≤200 rows per POST**; the route chunks
internally at 6 rows/insert. Response: `{ "accepted": [<id>, …] }`. The producer
treats any returned id as "accepted"; in the streamlined model this is just
confirmation the batch landed.

`filed_at` fallback: if not set directly, parse it out of `filing_metadata.filed_at`
(mirrors `to_ingest_record`).

### 3.3 The `id` echo token

The ingest route reads `id: number`, echoes it in `accepted`, and **does not**
use it as the PK (D1 assigns UUIDv7 on insert; existing id kept on conflict). The
Python producer sent its SQLite rowid. With no SQLite, the Rust producer assigns
a **monotonic per-process `u64` counter** purely as an echo token. It has no
persistence semantics and resets each process start — which is fine, since the
real identity is `(accession, doc_type, filename)` (the UNIQUE key) and the D1 id.

### 3.4 D1 schema (unchanged — `frontend/src/db/schema.ts`)

Table `exhibits`, all columns `text`, `id` PK (UUIDv7 assigned by ingest),
`UNIQUE(accession, doc_type, filename)`, indexes on `filed_at`, `form_type`,
`cik`. The new D1 is created and migrated with the **existing** drizzle
migrations (`frontend/migrations/0000_*.sql` + `0001_*.sql`) so it is identical
to production, just empty.

### 3.5 Filing header (`filing_metadata` JSON, parity with `extract_filing_header`)

Always the full key set, empty defaults for missing data:

```json
{ "company_name":"", "cik":"", "sic":"", "state_of_incorporation":"",
  "period":"", "filing_date":"", "filed_at":"", "file_number":"",
  "location":"<city, state>", "items":[] }
```

Source = SEC `<SEC-HEADER>` SGML, parsed via `secinfra`'s
`ParsedSubmissionMetadata` (the standardized key/value event stream). Map the
standardized keys to the fields above. `filed_at` = `acceptance-datetime`
(`YYYYMMDDHHMMSS`, ET — the real acceptance time). `location` joins city + state
with `", "`, skipping blanks. Never errors → on any parse failure return the
empty-default header.

### 3.6 Markdown (`converter.py` → Rust)

- HTML exhibits → `htmd` crate (HTML→Markdown). Output trimmed.
- PDF/DOCX exhibits → not converted (out of scope); they yield no text →
  `empty` status, and the image path catches scanned/image-only ones.
- Empty/None input → `""`. Conversion failure → `""` (never panics).
- Status: non-empty result → `done`; fetched-but-empty result → `empty`;
  fetch/convert error → `error`.

### 3.7 Images (`images.py` → Rust, inline)

For an exhibit whose markdown is **image-only** (markdown has image refs and,
after stripping image refs + labels, no readable text survives — port
`is_image_only` + the `clean_excerpt` strip), capture its images:

1. Image-ref filenames are scraped from the markdown with the regex
   `\(\s*([^()\s"]+\.(?:jpe?g|png|gif|tiff?|svg|webp))(?:\s+"[^"]*")?\s*\)` (case-insensitive),
   deduped, in order.
2. Pull `GRAPHIC` documents from the same filing's SGML (`secinfra` exposes
   document `doc_type`/`filename`/`content`, uudecoding handled by the crate),
   restricted to the filenames referenced by **this** exhibit (`only` set).
3. Upload all of a filing's selected images in **one HF commit** to
   `SEC_IMAGE_REPO` (default `arthrod/sec-ex10-exhibits`) under
   `images/{accession}/{filename}`; public URL =
   `https://huggingface.co/datasets/{repo}/resolve/main/{path}`.
4. Put the URL list (JSON string) in `image_urls`. No `HF_TOKEN` → no-op
   (omit `image_urls`; enrich-only upsert leaves any existing value untouched).
   Any SEC/HF failure is logged and swallowed.

HF upload uses the HF Hub commit HTTP API directly (preupload + commit) with
`reqwest`, since there is no official Rust HF client. This is the trickiest port;
isolate it behind a trait so the pure capture logic is testable without network.

## 4. Rust crate layout (`backend-rust/`, package `sec-ex10-rust`)

| Module | Responsibility | Ported from |
|---|---|---|
| `config.rs` | env → `Config` (section 6) | `config.py` |
| `pipeline.rs` | `Monitor` stream → bounded `for_each_concurrent`; per-submission orchestration | `listener.py` + `worker.py` |
| `extract.rs` | build SGML `.txt` URL, fetch, `ParsedSgml::parse`, gather EX-10 + GRAPHIC docs | `listener._datamule_extractor` |
| `classify.rs` | EX-10 predicate (pure) | `parsing.classify_documents` |
| `header.rs` | `ParsedSubmissionMetadata` → header JSON (pure mapping) | `parsing.extract_filing_header` |
| `markdown.rs` | HTML/PDF → markdown; status decision | `converter.py` |
| `images.rs` | ref scan, image-only test, GRAPHIC select, HF commit upload | `images.py` |
| `ingest.rs` | record build, batch, `POST /api/ingest`, retry queue | `d1_sync.py` |
| `health.rs` | axum `GET /health` → `{status:"ok", total_seen:<n>}` | `api.py` (health only) |
| `main.rs` | wire config, spawn health server + pipeline, signal handling | `worker._run_all` |

### 4.1 Pipeline behavior

- `secinfra::Monitor::new().use_rss(true).use_efts(true).polling_interval_ms(SEC_POLL_INTERVAL_MS).build()`
  yields `Vec<Submission>` batches; the built-in `AccessionCache` (50k LRU)
  dedups, replacing the Python `is_accession_seen` table.
- Each new `Submission` is processed with bounded concurrency
  (`SEC_CONCURRENCY`, default 8) via `futures::stream::for_each_concurrent`.
- Per submission: fetch SGML once → parse → for each EX-10 doc: convert markdown,
  capture images if image-only; compute filing header once per submission; build
  records; enqueue.
- A push task drains the record queue in batches of `SEC_PUSH_BATCH` (≤200) to
  `/api/ingest`. On HTTP failure, the batch is re-queued (bounded retry) and the
  pipeline keeps running — failures never stop the loop (parity with
  `_d1_push_loop` swallowing errors).
- Robustness rule from Python preserved everywhere: **one bad filing, network
  blip, or conversion error must never stop the loop.** Per-submission work is
  wrapped so errors are logged and skipped.

### 4.2 No "finalized" gate

The Python design buffered rows in SQLite and pushed only "finalized" ones
(markdown terminal + metadata present). With no buffer, each submission is
processed to completion in one pass (markdown + metadata + image attempt) and
pushed once. Image capture failure is non-blocking: `image_urls` is omitted and
the enrich-only upsert (`coalesce(excluded.image_urls, image_urls)`) preserves
any value already in D1, so re-pushes after a Space restart are safe and additive
— matching the decoupled-image semantics of the live system.

## 5. `markdown_status` state machine (preserved)

`done` — non-empty markdown produced.
`empty` — document fetched but no readable text (→ triggers image capture path).
`error` — fetch or conversion threw.

These are exactly the three values the frontend already renders.

## 6. Config / env (HF Space secrets & vars)

| Env | Default | Meaning |
|---|---|---|
| `SEC_USER_AGENT` | `secinfra` default UA | SEC User-Agent (secinfra `sec_user_agent()` reads this) |
| `D1_INGEST_URL` | — (set to new worker URL) | ingest endpoint to POST |
| `SEC_API_KEY` | — | shared ingest auth; also the Worker secret |
| `HF_TOKEN` | — | enables image upload; absent → images no-op |
| `SEC_IMAGE_REPO` | `arthrod/sec-ex10-exhibits` | HF dataset for image blobs |
| `SEC_POLL_INTERVAL_MS` | `200` | Monitor RSS poll interval |
| `SEC_CONCURRENCY` | `8` | in-flight submissions |
| `SEC_PUSH_BATCH` | `100` | rows per ingest POST (hard cap 200) |
| `PORT` | `7860` | health server port (HF `app_port`) |

## 7. Deploy artifacts

- **`backend-rust/Dockerfile`** — multi-stage: `rust:1-slim` (or `rust:1-bookworm`)
  builder with `build-essential`/`cc` for the crate's C SGML vendor `build.rs`;
  runtime `debian:bookworm-slim`, non-root uid 1000, `EXPOSE 7860`,
  `CMD ["sec-ex10-rust"]`.
- **`backend-rust/README.md`** — HF Space front-matter (`sdk: docker`,
  `app_port: 7860`, title/emoji).
- **`backend-rust/Cargo.toml`** — `secinfra` as a **path dependency** to the
  vendored copy (`secinfra = { path = "vendor/secinfra" }`), plus `tokio`,
  `reqwest` (rustls), `axum`, `serde`/`serde_json`, `htmd`, `tracing`, `futures`,
  `anyhow`. The vendored `secinfra` keeps its own `Cargo.toml` and `build.rs`.
- **`frontend/wrangler.v2.jsonc`** — copy of `wrangler.jsonc` with:
  `name: "sec-ex10-frontend-v2"`, the **new** D1 `database_id`, **no**
  `routes`/custom_domain block (use the `*.workers.dev` URL), its own
  `SESSION` KV (or reuse — KV is read-mostly for sessions; a fresh namespace is
  cleaner). The live `wrangler.jsonc` is **not edited**.
- **New D1** created via `wrangler d1 create sec-ex10-v2`; apply
  `frontend/migrations/*` to it.

### 7.1 Deploy steps (documented in `backend-rust/README.md` + spec)

1. `wrangler d1 create sec-ex10-v2` → capture id → put in `wrangler.v2.jsonc`.
2. Apply migrations to the new D1 (`wrangler d1 migrations apply sec-ex10-v2 --remote`
   or `d1 execute --file`).
3. `cd frontend && bun run build && wrangler deploy -c wrangler.v2.jsonc` →
   note the `sec-ex10-frontend-v2.<subdomain>.workers.dev` URL.
   `wrangler secret put SEC_API_KEY -c wrangler.v2.jsonc`.
4. Create HF Space `arthrod/sec-ex10-api-rust` (docker). Set Space secrets:
   `SEC_API_KEY` (same value), `D1_INGEST_URL` (the v2 worker URL + `/api/ingest`),
   `HF_TOKEN`, `SEC_USER_AGENT`. Push `backend-rust/` to the Space repo (or
   build-context upload).
5. Verify: Space `/health` 200; new worker site loads; rows appear in `sec-ex10-v2`
   as filings stream.

## 8. Testing

- **Rust unit tests** (no network) for pure logic: `classify` (EX-10 vs EX-101
  vs other), `header` mapping (standardized events → JSON, missing-field
  defaults, malformed input → empty header), image-ref regex + `is_image_only`,
  ingest record shape (`filed_at` fallback, key names, JSON-string encoding of
  `filing_metadata`/`image_urls`).
- **Seams** for I/O (like Python's injected `extractor`/`poster`/`uploader`):
  the SGML fetcher, the ingest poster, and the HF uploader are traits with real
  and fake impls so the pipeline is testable without SEC/HF/D1.
- No live SEC calls in CI.

## 9. Out of scope / known deltas

- **No SQLite durability.** In-memory dedup only; restart re-streams recent
  filings and re-pushes (idempotent — harmless).
- **Markdown engine differs.** `htmd` (HTML-only) replaces `markitdown`; PDF and
  DOCX exhibits (rare for EX-10) are not converted → `empty` status (image path
  still catches scanned/image-only exhibits).
- **No DNS cutover.** Live `live-contracts.arthur.law` and the Python Space are
  untouched. Cutover (point the route at the v2 worker, retire the Python Space)
  is a separate, later change.
- **No new HF dataset.** Image blobs reuse the existing `arthrod/sec-ex10-exhibits`.
- The local `all_exhibits` (non-EX-10) table is not reproduced — it was never
  part of the D1 contract.

## 10. Risks

- **C SGML vendor build** (`secinfra` `build.rs` + `cc`) must compile in the
  Docker builder — needs `build-essential`. Verified as a build step early.
- **HF commit API in raw Rust** (no official client) is the highest-effort
  port; isolate behind a trait, test the URL/blob assembly, accept that live
  verification happens against the real dataset.
- **`htmd` output fidelity** vs markitdown may differ in whitespace/structure;
  acceptable since the frontend renders markdown loosely and previews are
  cleaned at read time.
