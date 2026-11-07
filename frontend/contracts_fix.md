# Restore /search: external-content FTS5 on D1 — implementation plan

**Goal:** `/search` (HTML), `Accept: text/markdown` search, and the MCP `search_agreements` tool return real full-text results over ~19.5k EX-10 exhibits (1.8 GB of markdown) with exact totals, newest-first ordering, and `<mark>` snippets, without scanning markdown at request time and without a second search system.

**Stack:** Astro 7.4.5 SSR on Cloudflare Workers (`@astrojs/cloudflare` 14), D1, SQLite FTS5, Drizzle, Vitest 4 in the Workers pool, Wrangler, pnpm.

**Execution:** three stacked branches via `gh stack`, one task each. Wait for CI only on the last task. Every acceptance item (AC) names an independent measurer: a command or probe that proves the behaviour from outside the diff. Record RED before implementing and GREEN after.

**Supersedes:** `contracts_fix.v1.md` (kept for reference). This rewrite drops the maintenance flag, the checkpoint state machine, the vocabulary table, and the hand-made Drizzle snapshot, and fixes a query shape that would have failed the v1 latency gate. Reasons are in §1.

---

## 0. Verified facts (2026-11-07, read-only probes against production and local D1)

| Fact | Value | How measured |
|---|---|---|
| Migration ledger | `0000_flaky_avengers.sql`, `0001_bumpy_tusk.sql` (with `.sql`) | `SELECT id, name FROM d1_migrations` on remote `sec-ex10` |
| 0002 DDL | Already applied, not recorded: 18 columns incl. `source`, `size_bytes`, `detected_at`; `idx_ex_detected_at` exists | `PRAGMA table_info(exhibits)` (works remotely), `sqlite_master` |
| FTS objects | None | `sqlite_master` |
| Rows / rowids | 19,554 rows, `rowid` 1..19,554 contiguous | `SELECT count(*), min(rowid), max(rowid) FROM exhibits` |
| DB size | 1,835,937,792 bytes | `meta.size_after` |
| `sqlite_version()` | Blocked on D1 (`not authorized to use function`) | remote execute |
| Current failure | HTML 500, markdown 500, MCP `isError: true` with `Failed query: select count(*) from exhibits_fts …` | curl |
| LIKE fallback | Dead code: `DrizzleQueryError.message` never contains `no such table`, so the catch always rethrows | MCP error text |
| `drizzle-kit check` | Passes with no `0003_snapshot.json` | `pnpm exec drizzle-kit check` |
| Local D1 (miniflare) FTS5 | External-content tables, `content_rowid='rowid'`, `snippet()` on column 1, `delete-all`, `rebuild`, `integrity-check` with `rank = 1`, and `rowid IN (subquery) AND MATCH` all work | wrangler `--local` probes |
| Trigger-guard measurer | Across separate transactions, `max(id) FROM exhibits_fts_data` is unchanged after an unchanged or NULL→NULL re-push and increases after a real change | wrangler `--local` probes |
| Triggers on shadow tables | Forbidden (`cannot create triggers on shadow tables`) | wrangler `--local` probe |
| `meta.rows_written` locally | Not reported by miniflare; do not use as a test measurer | wrangler `--local` probe |

---

## 1. Design decisions

### 1.1 Index: external-content FTS5 keyed on `exhibits.rowid`

```sql
CREATE VIRTUAL TABLE `exhibits_fts` USING fts5(
  `description`,
  `markdown`,
  content = 'exhibits',
  content_rowid = 'rowid',
  tokenize = 'unicode61 remove_diacritics 2'
);--> statement-breakpoint
CREATE TRIGGER `exhibits_fts_ai` AFTER INSERT ON `exhibits` BEGIN
  INSERT INTO `exhibits_fts`(`rowid`, `description`, `markdown`)
  VALUES (new.`rowid`, coalesce(new.`description`, ''), coalesce(new.`markdown`, ''));
END;--> statement-breakpoint
CREATE TRIGGER `exhibits_fts_ad` AFTER DELETE ON `exhibits` BEGIN
  INSERT INTO `exhibits_fts`(`exhibits_fts`, `rowid`, `description`, `markdown`)
  VALUES ('delete', old.`rowid`, coalesce(old.`description`, ''), coalesce(old.`markdown`, ''));
END;--> statement-breakpoint
CREATE TRIGGER `exhibits_fts_au` AFTER UPDATE OF `description`, `markdown` ON `exhibits`
WHEN new.`description` IS NOT old.`description` OR new.`markdown` IS NOT old.`markdown` BEGIN
  INSERT INTO `exhibits_fts`(`exhibits_fts`, `rowid`, `description`, `markdown`)
  VALUES ('delete', old.`rowid`, coalesce(old.`description`, ''), coalesce(old.`markdown`, ''));
  INSERT INTO `exhibits_fts`(`rowid`, `description`, `markdown`)
  VALUES (new.`rowid`, coalesce(new.`description`, ''), coalesce(new.`markdown`, ''));
END;
```

- No `IF NOT EXISTS`: applying against a database that already holds the old standalone table must fail loudly, never silently keep the wrong schema.
- No `fts5vocab` table. Index membership is `SELECT count(*) FROM exhibits_fts_docsize`; correctness is `integrity-check` with `rank = 1`.
- The `IS NOT` guard matters: ingest rewrites `description` and `markdown` on every conflict, so without it every re-push re-tokenizes ~94 KB.
- `exhibits` has a TEXT primary key, so `rowid` is implicit. A `0001`-style rebuild (create, copy, drop, rename) renumbers rowids and drops the triggers. Any future migration that rebuilds `exhibits` must recreate the triggers and run `rebuild`. D1 does not expose `VACUUM`.

### 1.2 Query: page rowids first, snippets only for the page

`snippet()` in the select list of an `ORDER BY … LIMIT` query is evaluated for every matching row before the sort. For broad terms (`agreement` matches most of the corpus) that reads and tokenizes gigabytes per request and cannot meet any latency gate. The query is therefore split:

```sql
-- statement 1: exact total (index only, no content read)
SELECT count(*) AS n FROM exhibits_fts WHERE exhibits_fts MATCH ?1;

-- statement 2: page rowids by filed_at, then snippets for those rows only
SELECT <summaryCols>, snippet(exhibits_fts, 1, '<mark>', '</mark>', '…', 28) AS snippet
FROM exhibits_fts f JOIN exhibits e ON e.rowid = f.rowid
WHERE exhibits_fts MATCH ?1
  AND f.rowid IN (
    SELECT f2.rowid FROM exhibits_fts f2 JOIN exhibits e2 ON e2.rowid = f2.rowid
    WHERE exhibits_fts MATCH ?1
    ORDER BY e2.filed_at DESC LIMIT ?2 OFFSET ?3)
ORDER BY e.filed_at DESC;
```

Both statements go in one `db.batch` (one round trip). The inner query reads only `filed_at`, which precedes `markdown` in the record, so no overflow pages are touched for non-page rows. `summaryCols` (including `substr(markdown, 1, 2000)`) is unchanged because `DocketRow` derives titles from it. Markdown is FTS column 1.

### 1.3 Rollout order: migrate and backfill under the old Worker, then deploy

The old Worker's search already fails against the new schema (`no such column: exhibits_fts.id`), so applying `0003` and backfilling first keeps search at its current 500 instead of serving partial results, with zero maintenance-mode code. The only barrier needed is the external producer being stopped, verified by an unchanged `(count(*), max(rowid))` signature. Rollback of the code is `wrangler rollback`; the triggers keep the index correct under either Worker.

### 1.4 Backfill: try `rebuild` once, else fixed rowid ranges

`INSERT INTO exhibits_fts(exhibits_fts) VALUES('rebuild')` reindexes the whole content table in one atomic statement. If it exceeds D1's 30-second statement budget it rolls back and the index stays empty, so trying it costs nothing. If it fails, a 60-line script walks `rowid` ranges of fixed size with a signature check before each chunk. There is no checkpoint file: the script prints the last completed upper bound, and the operator resumes with `--from`. Recovery from any uncertain chunk is `delete-all` and restart; at 19.5k rows that is cheaper than any state machine.

### 1.5 What the Astro expert skills contribute (reviewed `astro-7`, `astro-content`, `astro-deployment`, `astro-islands`, `astro-starlight`, `solid-astro`)

- **Pagefind and DocSearch are explicitly "static bundle, small-medium sites" and "Algolia servers, large sites".** Neither can index live D1 rows at request time. Request-time FTS5 in D1 stays the only fit. Content collections and loaders are build-time; rejected for the same reason.
- **Astro 7 route cache** (`cache.provider`, `Astro.cache.set({ maxAge, swr, tags })`, experimental `cacheCloudflare()`): the existing `Cache-Control: public, max-age=30, stale-while-revalidate=60` on `/search` already covers the hot path. Tag-based invalidation on ingest is a follow-up, not part of this fix.
- **Server islands (`server:defer`)** could stream results below a static shell. Not needed once snippets are computed per page; listed as a follow-up.
- **Cloudflare adapter**: bindings via `import { env } from 'cloudflare:workers'`, already how `ingest.ts` and `mcp.ts` work. Astro 7.4.5 with adapter 14.1 matches the skill's version matrix.
- **SOLID split**: search leaves `src/lib/api.ts` (319 lines) for its own module `src/lib/search.ts`. `api.ts` re-exports it so callers do not change. The repo does not adopt the 100-line ceiling or `src/interfaces/` wholesale.

---

## 2. Global constraints

- Preserve the `ex10Search(query, page, pageSize, db)` contract: exact totals, pagination, `filed_at DESC`, `<mark>` snippets, `engine: 'fts'`.
- No `LIKE` against `exhibits.markdown`, no Pagefind, no external search service, no `filing_metadata` indexing.
- Do not change homepage ordering or recreate `idx_ex_filed_newest`.
- Run Wrangler from `frontend/` with the top-level `wrangler.jsonc`. Never `--env staging` for production. `--dry-run` before every deploy.
- Never join FTS on the UUID. Never use `NOT EXISTS` against `exhibits_fts` (external content forwards non-MATCH reads to `exhibits`).
- Abort the rollout if `size_after` exceeds 8 GB at any point.

---

## 3. Task 1 — schema and query

**Branch:** `feat/fts-1-schema-query` on `main`.

**Files**
- Modify `migrations/0003_exhibits_fts.sql` (replace with §1.1; keep `--> statement-breakpoint`).
- Create `src/lib/search.ts`: `ex10Search`, `SearchResult`, the batch query from §1.2. Export `summaryCols`, `orderNewest`, `pages` from `api.ts` (or move them to `src/lib/columns.ts`) so both modules share them.
- Modify `src/lib/api.ts`: delete `ex10Search`, the LIKE path, and `'like'` from `SearchResult.engine`; `export { ex10Search, type SearchResult } from './search'`.
- Modify `test/seed.ts`: drop `exhibits_fts` then `exhibits` before re-applying migrations.
- Modify `test/d1-read.test.ts`: keep the existing search cases; add a two-page pagination case and a description-only match case.
- Create `test/fts-external.test.ts` (cases below).
- Modify `test/ingest.test.ts`: conflict re-push through `POST` then search.
- Modify `README.md`: search section, migration note, remove the LIKE fallback sentence.
- Add `snippetHtml` to `src/lib/summary.ts` and use it in `src/components/DocketRow.astro` (found during the AC 1.11 browser check).

**Steps**
1. Run the AC measurers below against `main` and record RED (new tests absent or failing; `grep LIKE` finds the fallback).
2. Write the new and modified tests first. Run `pnpm vitest run test/d1-read.test.ts test/fts-external.test.ts test/ingest.test.ts` and record the failures.
3. Replace the migration, write `src/lib/search.ts`, strip `api.ts`, update `seed.ts`.
4. Run the full suite, `pnpm check`, `pnpm build`, `pnpm lint`, `pnpm fmt:check`.
5. Browser check: `pnpm dev`, open `/search?q=leasing` in Chrome, confirm results with highlighted terms and working next/prev links.
6. Commit; `gh stack add`, `gh stack push`. Do not wait for CI.

**Acceptance items**

| AC | Statement | Measurer | Expected |
|---|---|---|---|
| 1.1 | Rows that existed before the triggers become searchable after a range backfill | `test/fts-external.test.ts`: apply `0000`–`0002`, insert two rows, apply `0003`, run `INSERT INTO exhibits_fts(rowid, description, markdown) SELECT rowid, coalesce(description,''), coalesce(markdown,'') FROM exhibits WHERE rowid > 0 AND rowid <= 50`, call `ex10Search('leasing')` | `total === 2`, items newest first, `snippet` contains `<mark>leasing</mark>` |
| 1.2 | Index matches base content after inserts, updates, deletes | same file: after each mutation run `INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', 1)` | resolves without error |
| 1.3 | The full check is a real gate | same file, negative fixtures: (a) insert the same rowid into `exhibits_fts` twice; (b) `DROP TRIGGER exhibits_fts_au` then update markdown | `integrity-check` with `rank = 1` rejects both; `rank = 0` must not be relied on |
| 1.4 | Unchanged re-pushes do not rewrite the index | same file: read `SELECT max(id) FROM exhibits_fts_data`, run the real ingest `POST` with identical `description` and `markdown`, read again; then a changed re-push; then a NULL→NULL description re-push | unchanged and NULL cases: equal; changed case: greater |
| 1.5 | Broad terms do not compute snippets outside the page | same file: insert 300 rows whose markdown is ~40 KB and all contain `agreement`; `ex10Search('agreement', 2, 12)` | `total === 302` (fixtures + 300), 12 items, every item has a non-empty `snippet`, `page_size === 12`, `total_pages === 26`; `searchPageSql()` puts `snippet(exhibits_fts, 1, …)` in the outer select and `LIMIT ? OFFSET ?` inside the `rowid IN (SELECT …)` subquery (D1 exposes no per-call snippet counter, so the SQL shape is the measurer, as `listEx10OrderSql` already does for the list) |
| 1.6 | Operators and phrases behave as before | `test/d1-read.test.ts` existing cases | `'leasing OR *'` → 0; `"contract body"` → `['1']`; `"body contract"` → 0 |
| 1.7 | No LIKE path remains | `grep -n "LIKE\|'like'" src/lib/search.ts src/lib/api.ts` | only the filer filter in `listEx10` (`json_extract … LIKE`) matches |
| 1.8 | Migration text is what §1.1 says | `grep -c "content = 'exhibits'\|content_rowid = 'rowid'\|IS NOT old" migrations/0003_exhibits_fts.sql` | 3; `grep -c "IF NOT EXISTS"` → 0; `grep -c UNINDEXED` → 0 |
| 1.9 | Seed loop is idempotent | `test/fts-external.test.ts`: call `seed()` twice in one test, then `integrity-check` with `rank = 1` | passes |
| 1.10 | Repo gates | `pnpm vitest run && pnpm check && pnpm build && pnpm lint && pnpm fmt:check` | all exit 0 |
| 1.10b | Snippet highlights render | `test/summary.test.ts` `snippetHtml` cases | `<mark>` survives marker stripping (the old card code deleted every `>`, so highlights rendered as literal `<markAGREEMENT</mark`); table-rule dashes are dropped; other markup is escaped |
| 1.11 | Visible in a browser | Chrome on `pnpm dev`: `/search?q=leasing`, `/search?q=%22contract%20body%22` | result cards with `<mark>` highlights, pagination links present, no console errors |

---

## 4. Task 2 — backfill script

**Branch:** `feat/fts-2-backfill` on `feat/fts-1-schema-query`.

**Files**
- Create `scripts/fts-backfill-core.ts`: pure functions, no Node imports: `parseArgs(argv)`, `planRanges(from, to, chunk)`, `sameSignature(a, b)`, `runBackfill(deps, opts)` where `deps = { exec(sql): Promise<{ changes, sizeAfter, durationMs }>, probe(url): Promise<number>, log(line) }`.
- Rewrite `scripts/fts-backfill.ts`: CLI wrapper. `node:child_process.execFile('pnpm', ['exec', 'wrangler', 'd1', 'execute', db, flag, '--json', '--command', sql])` with an argument array, never a shell string. `probe` is `fetch(origin + '/stats.json')`.
- Modify `package.json`: `"db:fts-backfill": "node scripts/fts-backfill.ts"`; drop the bun shebang.
- Create `test/fts-backfill.test.ts` (Workers pool, injected deps, no child processes).
- Modify `README.md` runner section and the comment block at the top of `migrations/0003_exhibits_fts.sql`.

**Behaviour**
- Flags: `--remote | --local` (required), `--to <rowid>` (required with `--remote`; the `max(rowid)` snapshot taken after the producer stopped), `--from <rowid>` (default 0), `--chunk <n>` (default 50, integer 1..500), `--db <name>` (default `sec-ex10`), `--origin <url>` (default `https://live-contracts.arthur.law`, `--local` skips the probe).
- Before the first chunk: snapshot `SELECT count(*) AS n, max(rowid) AS m FROM exhibits`. Before every chunk: re-read and compare. Any difference → exit 2 with `SIGNATURE_CHANGED: producer wrote during backfill. Run delete-all and restart from 0.`
- Chunk statement: `INSERT INTO exhibits_fts(rowid, description, markdown) SELECT rowid, coalesce(description,''), coalesce(markdown,'') FROM exhibits WHERE rowid > ?lo AND rowid <= ?hi` with literal integers.
- After every chunk: log `hi`, `changes`, `durationMs`, `sizeAfter`; probe `/stats.json`, retry 3× with 1/2/4 s backoff; non-200 → exit 3. `sizeAfter > 8 GB` → exit 5. Chunk error → exit 4 and print the recovery commands. The last log line of a successful run is `DONE to=<hi>`.
- No parallel chunks. No checkpoint file. Resume is `--from <last logged hi>`. Before any write the script checks that `count(*) FROM exhibits_fts_docsize` equals `count(*) FROM exhibits WHERE rowid <= from` (0 for a fresh run). A chunk that committed but was never logged fails this check (exit 7), so recovery is always `INSERT INTO exhibits_fts(exhibits_fts) VALUES('delete-all')` then a run from 0. At the end the document count must equal the row count (exit 6 otherwise).
- `--to` must equal the snapshot's `max(rowid)`; a mismatch means the producer is still writing (exit 2).
- Exit codes: 0 done, 2 signature changed, 3 unhealthy probe, 4 chunk failed, 5 size cap (or a remote write without `size_after`), 6 count mismatch, 7 index state (at resume or after any chunk), 8 integrity-check failed, 9 a D1 read failed, 64 usage. Every successful run ends with `integrity-check rank=1` (#83).
- `scripts/tsconfig.json` type-checks the backfill files with `@types/node` (added as a dev dependency). Runs as `node scripts/fts-backfill.ts`; Node 26 strips types natively.

**Steps**
1. Record RED: `test/fts-backfill.test.ts` absent; `grep Bun scripts/fts-backfill.ts` finds `Bun.spawn`; `grep "NOT EXISTS" scripts/fts-backfill.ts` finds the forbidden membership test.
2. Write the core tests (AC 2.1–2.5), run, record failures.
3. Implement core, then the CLI wrapper, then package script.
4. Local end-to-end: seed a throwaway `--persist-to` directory with `0000`–`0003` and 7 rows, run `pnpm db:fts-backfill -- --local --to 7 --chunk 3`, then run `integrity-check` with `rank = 1` through the same `--persist-to`.
5. `pnpm vitest run`, `pnpm lint`, `pnpm fmt:check`. Commit; `gh stack add`, `gh stack push`.

**Acceptance items**

| AC | Statement | Measurer | Expected |
|---|---|---|---|
| 2.1 | Ranges are exact and ascending | `planRanges(0, 7, 3)` | `[[0,3],[3,6],[6,7]]`; `planRanges(7, 7, 3)` → `[]` |
| 2.2 | Invalid configuration fails closed before any write | `parseArgs` with `--chunk 0`, `-1`, `2.5`, `501`, missing `--to` with `--remote` | throws; `deps.exec` never called |
| 2.3 | Signature change stops the run | `runBackfill` with a `probe`/`exec` stub whose second signature read returns `m + 1` | resolves `{ code: 2 }`, exactly one chunk executed |
| 2.4 | Unhealthy probe stops the run after bounded retries | `probe` stub returns 500 four times | `{ code: 3 }`, probe called 4 times, no further chunk |
| 2.5 | Resume continues from `--from` and finishes with `DONE` | `runBackfill` with `from = 6, to = 7, chunk = 3` | one chunk `(6,7]`, last log line `DONE to=7` |
| 2.6 | Real CLI path against local D1 is correct and idempotent to rerun after reset | Step 4 above, then `delete-all`, run again | both runs log `DONE to=7`; `SELECT count(*) FROM exhibits_fts_docsize` → 7; `integrity-check` with `rank = 1` passes after each run |
| 2.7 | No shell string, no Bun, no NOT EXISTS | `grep -n "Bun\.\|NOT EXISTS\|exec(\`\|shell: true" scripts/` | no matches |
| 2.8 | Repo gates | `pnpm vitest run && pnpm lint && pnpm fmt:check` | exit 0 |

---

## 5. Task 3 — production rollout and verification

**Branch:** `feat/fts-3-rollout` on `feat/fts-2-backfill`. CI must be green on this task before publishing.

**Files**
- Modify `test/smoke-prod.sh`: add `search page with query` (`/search?q=engagement` → 200 and body contains `<mark`), `markdown search` (`Accept: text/markdown` → 200 and body contains `engagement`), and `search_agreements` via the existing `rpc` helper (`.result.isError != true`, `.result.content[0].text | fromjson | .total > 0`).
- Modify `README.md` and this file with measured numbers only.

**Runbook (operator steps, each with its check; stop on any failed check)**

| # | Action | Check |
|---|---|---|
| 0 | Record the current Worker version: `pnpm exec wrangler deployments list` | version id noted as rollback target |
| 1 | Merge Tasks 1 and 2 to `main`. Do not deploy. | `git log origin/main -1` contains Task 2 |
| 2 | Stop the external producer (HF worker). | `SELECT count(*), max(rowid) FROM exhibits` twice, 60 s apart → identical. Note `M = max(rowid)` |
| 3 | Time Travel bookmark: `pnpm exec wrangler d1 time-travel info sec-ex10` | bookmark recorded |
| 4 | Ledger: `INSERT INTO d1_migrations(name, applied_at) VALUES ('0002_boring_union_jack.sql', CURRENT_TIMESTAMP)` | `pnpm exec wrangler d1 migrations list sec-ex10 --remote` shows only `0003_exhibits_fts.sql` |
| 5 | `pnpm exec wrangler d1 migrations apply sec-ex10 --remote` (confirm only 0003) | `sqlite_master` has `exhibits_fts` with `content='exhibits'`, triggers `exhibits_fts_ai/ad/au`, `exhibits_fts_au.sql` contains `IS NOT`; `SELECT count(*) FROM exhibits_fts_docsize` → 0 |
| 6 | Try one-shot: `INSERT INTO exhibits_fts(exhibits_fts) VALUES('rebuild')` (time it) | success → docsize count = 19,554, skip step 7. Error → docsize count still 0 (rolled back), continue |
| 7 | `pnpm db:fts-backfill -- --remote --to <M> --chunk 50`. If any chunk exceeds 10 s, rerun from the last `hi` with `--chunk 20`. | last line `DONE to=<M>`; docsize count = 19,554; `size_after` ≤ 8 GB |
| 8 | `INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', 1)` (time it); signature re-read | passes; signature equals step 2 |
| 9 | Direct probes: `SELECT count(*) FROM exhibits_fts WHERE exhibits_fts MATCH '"engagement"'` and `'"agreement"'` | both > 0; `engagement` ≪ `agreement` |
| 10 | `pnpm build && pnpm exec wrangler deploy --dry-run` then `pnpm exec wrangler deploy` | deploy output names `sec-ex10-frontend` and the production D1 id `dfb55595…` |
| 11 | AC 3.1–3.3 below | all pass |
| 12 | Restart the producer. Watch five ingest batches. | each 200, p95 < 10 s, no `overloaded` errors; after the batches, `integrity-check` with `rank = 1` passes and a term from a new filing matches |
| 13 | Record numbers in README and here; commit; `gh stack add`, `gh stack push`; wait for CI (10-minute cap). | CI green |

**Rollback**
- Worker misbehaves: `pnpm exec wrangler rollback` to the step 0 version. Search returns to its current 500; the triggers keep the index correct under the old Worker, so no database action is needed.
- Migration or backfill damaged the database: keep the producer stopped, `pnpm exec wrangler d1 time-travel restore sec-ex10 --bookmark <step 3>`, verify `sqlite_master` has no FTS objects and the ledger equals step 4's pre-state, then restart the producer. Replay its retained backlog.

**Acceptance items**

| AC | Statement | Measurer | Expected |
|---|---|---|---|
| 3.1 | Selective and broad searches are fast and correct | five sequential `curl -A "Mozilla/5.0" -w '%{http_code} %{time_total}'` per term: `engagement`, `CapForce`, `leasing`, `credit%20agreement`, `a`, `agreement` | every request 200; selective terms < 1 s; `a` and `agreement` < 5 s; selective bodies contain `<mark` |
| 3.2 | Markdown and MCP callers get real results | `H=https://live-contracts.arthur.law bash test/smoke-prod.sh` | exit 0 incl. the three new checks |
| 3.3 | Pagination is consistent | `/search?q=agreement&page=1` and `&page=2` | different first result ids; both 200; totals equal |
| 3.4 | Index and base agree after rollout and after resumed ingest | step 8 and step 12 integrity checks; `docsize count == exhibits count` | pass; equal |
| 3.5 | Public health held during backfill | `/` and `/stats.json` polled every chunk by the script | always 200 under 2 s |
| 3.6 | Storage headroom | final `size_after` | ≤ 8 GB, recorded in README |

---

## 6. Retained analysis

- **Root cause:** production never received `exhibits_fts`. Local Vitest applies every migration in `test/seed.ts`, so green tests never proved production was migrated. `search.astro`, the markdown middleware and `mcp.ts` all call the same `ex10Search`; HTML and markdown surface 500, MCP surfaces `isError: true`.
- **Ingest overload** (`D1 DB is overloaded`) is a separate, older incident on `POST /api/ingest`. This plan only reduces unnecessary FTS rewrites via the trigger guard and paces catch-up after the producer resumes.
- **`ftsQuery`** quotes every token so `AND`, `OR`, `NOT`, `*`, `^`, `:` and parentheses are literal or stripped; phrase order is meaningful. Unchanged.
- **Field boundaries:** only `description` and `markdown` are indexed. Filer names that exist only in `filing_metadata` JSON are out of scope.
- **Homepage ordering:** bare `ORDER BY filed_at` on `idx_ex_filed_at`. Do not restore `IS NULL`, `found_at` or `id` expressions.

## 7. Follow-ups (not in this plan)

- Astro 7 route cache with tag invalidation on ingest (`Astro.cache.set({ tags: ['search'] })`, `cacheCloudflare()` once stable).
- `server:defer` island for the result list if perceived latency on broad terms still matters after 3.1.
- Move CI (`.github/workflows/frontend.yml`) and `db:setup` from bun to pnpm.
- Index filer names from `filing_metadata` as a third FTS column.
