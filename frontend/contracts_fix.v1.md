# Fix live-contracts search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore full-text search for HTML, markdown, and MCP callers without scanning 1.8 GB of markdown or adding a second search system.

**Architecture:** Replace the unapplied standalone FTS5 migration with an external-content FTS5 index over `exhibits`. Join FTS hits to Drizzle's existing `exhibits` rows by SQLite `rowid`, backfill keyset-ordered chunks while ingest is paused, and keep D1 as the only authoritative content store.

**Tech Stack:** Astro SSR, Cloudflare Workers and D1, SQLite FTS5, Drizzle ORM, TypeScript, Vitest, Wrangler, pnpm.

**Spec:** Section 1 records the adopted design. Sections 2 and 3 contain the implementation plan and retained incident analysis.

**Execution:** Three dependent tasks form a `gh stack`. Do not wait for CI/CD except on the last task. This document does not apply migrations, run the backfill, deploy, commit, or push.

## Global constraints

- Preserve the existing `ex10Search(query, page, pageSize, db)` result contract, including exact totals, pagination, `filed_at` ordering, and marked snippets.
- Do not enable `LIKE` against `exhibits.markdown`, reintroduce Pagefind, or add an external search service.
- Do not run the current standalone `0003` migration. Rewrite it only while production has not recorded or applied it.
- Stop the external producer and enable an enforced `SEARCH_MAINTENANCE` flag before creating production triggers. Keep both barriers in place until backfill and integrity checks complete; maintenance mode must return an intentional unavailable result instead of partial search hits.
- Deploy the new rowid-based search code before applying the replacement `0003`.
- Run Wrangler from `frontend/` with the top-level `wrangler.jsonc`. Never use `--env staging` for the production rollout. Freeze all Worker deployments from Step 4 through final publication of Task 3, including CI/CD deployment stages; read-only CI checks may continue. Only the designated rollout operator may change `SEARCH_MAINTENANCE` or publish during that window.
- Do not change homepage ordering or recreate `idx_ex_filed_newest`.

## Review focus

- Drizzle emits inserts and upserts with every schema column, and ingest assigns `description` and `markdown` on each conflict update. Test the real migration chain and real D1 `db.batch`, not a shortened SQLite fixture.
- External-content FTS forwards non-`MATCH` reads to `exhibits`. Do not use `NOT EXISTS`, ordinary row counts, or ordinary deletes to infer or manipulate index membership.
- A straggler update/delete against an unbackfilled row can abort its D1 statement. A straggler insert can instead be indexed by the AI trigger and silently inserted again by backfill. Enforce both producer barriers and compare the base-row signature throughout backfill.
- Ingest assigns both FTS columns on every conflict update. The update trigger needs a null-safe `IS NOT` guard so unchanged re-pushes do not tokenize the same body again.
- External content avoids a second markdown copy but still consumes index and document-size storage. Measure the [10 GB database limit and 30-second D1 request/batch budget](https://developers.cloudflare.com/d1/platform/limits/index.md) on a production-scale copy.
- Preserve `summaryCols` because `DocketRow` derives titles from `item.excerpt_md || item.excerpt`, even when it renders an FTS snippet as the card body.

---

## 1. Adopted option: external-content D1 FTS5

### Target schema

Replace `migrations/0003_exhibits_fts.sql` with the following statements. Keep the statement breakpoints used by Wrangler and `test/seed.ts`.

```sql
CREATE VIRTUAL TABLE `exhibits_fts` USING fts5(
  `description`,
  `markdown`,
  content = 'exhibits',
  content_rowid = 'rowid',
  tokenize = 'unicode61 remove_diacritics 2'
);--> statement-breakpoint
CREATE VIRTUAL TABLE `exhibits_fts_vocab` USING fts5vocab(`exhibits_fts`, 'instance');--> statement-breakpoint
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
  INSERT INTO exhibits_fts(rowid, description, markdown)
  VALUES (new.rowid, coalesce(new.description, ''), coalesce(new.markdown, ''));
END;
```

A D1 Vitest spike validated the core external-content design against migrations `0000` through `0002`, including Drizzle joins and counts, a D1 batch upsert, update/delete triggers, snippets, and `integrity-check`. Task 1 must test the final migration text, null-safe update guard, and vocabulary teardown through the repository's complete seed loop.

### Why this design replaces the original migration

- `content='exhibits'` stores the inverted index and document sizes without duplicating every markdown body in an FTS content table.
- FTS rowids now map directly to `exhibits.rowid`, so trigger operations do not scan for the old unindexed UUID text column.
- `snippet(exhibits_fts, 1, ...)` reads markdown through the external content table. Description is column 0 and markdown is column 1.
- `unicode61 remove_diacritics 2` preserves the intended tokenizer and behavior.
- `exhibits_fts_vocab` supports a cheap pre-backfill emptiness check and bounded diagnostics. It is not an index-membership table for normal application reads.

The UUID remains the public exhibit identifier and link target. It no longer participates in FTS joins.

### Required Drizzle changes

In `src/lib/api.ts`:

1. Change the join to `exhibits.rowid = exhibits_fts.rowid` using a `sql` fragment that Drizzle generates as valid SQL.
2. Change `snippet(exhibits_fts, 2, ...)` to `snippet(exhibits_fts, 1, ...)`.
3. Keep the count against `exhibits_fts MATCH ?`, pagination, `orderNewest`, and all existing `summaryCols` fields.
4. Delete the complete `LIKE` fallback and its catch. Let a missing or broken index throw rather than query `exhibits.markdown`.
5. Remove `'like'` from `SearchResult.engine` and update the search comment at `src/lib/api.ts:206`.
6. Update the fallback and backfill documentation in `README.md` and the migration's runner instructions.
7. Add optional `SEARCH_MAINTENANCE` to `Env` and default it to `"0"` in `wrangler.jsonc` so configuration-driven deploys do not silently clear it. In the ingest route, authenticate first, then reject with HTTP 503 when the flag is `1`, before body validation and before the `rows.length === 0` early return; unauthorized probes remain 401 and no paused request reaches D1. Make `ex10Search` throw a maintenance error, return intentional HTTP 503 responses from HTML and markdown search, and preserve MCP's existing `isError: true` JSON-RPC envelope. Normal operation requires an explicit `0`.

No Drizzle schema declaration is required for the virtual table. Continue accessing it through `sql` while selecting typed columns from `exhibits`.

### Rowid lifecycle rule

`exhibits` is a rowid table with a text primary key, not an `INTEGER PRIMARY KEY` alias. `rowid` values can change during operations such as a table rebuild or a SQLite `VACUUM`. Never run external-content maintenance after such an operation without rebuilding the index and revalidating triggers.

A `0001`-style base-table rebuild also drops the FTS triggers with the old table. After every migration, verify both rowid consistency and trigger presence; an existing FTS table alone proves nothing. Run `INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', 1)` after any restore, import, migration that rebuilds `exhibits`, or suspected rowid renumbering. Rebuild instead if the check fails. Keep the UUID join ban even after a rebuild.

---

## 2. Required review corrections and implementation tasks

### Production safeguards that precede every task

- Reconfirm the production ledger and schema instead of treating the last observed 19,554-row, two-migration state as current.
- Treat `sqlite_master.sql` as the authoritative object-definition probe. Use `PRAGMA table_info` and `PRAGMA index_info` only after confirming remote D1 accepts them; do not let an unsupported PRAGMA skip schema validation.
- Record a D1 Time Travel recovery checkpoint before the first production write. A deployment rollback does not restore database state. Confirm the producer retains enough local backlog to replay every post-checkpoint row.
- Rehearse on a production-scale copy with the real Drizzle ingest path. Record statement duration, `rows_read`, `rows_written`, `size_after`, search latency, the bounded FTS merge workload, and the full external-content integrity-check duration.
- Abort if `size_after` exceeds 8 GB at any point, preserving at least 2 GB under D1's hard cap.
- Tune chunk size against a maximum 2-second p95 D1 duration over the first 20 chunks and 2-second p95 homepage/stats responses. If chunk size 1 still exceeds those thresholds, stop.
- Calculate the full producer-pause estimate from the rehearsal. If it exceeds 2 hours, do not use this runbook; design a shadow-index/change-log rollout instead.
- Require five sequential production probes per search term: selective queries must return HTTP 200 in under 1 second and broad queries in under 5 seconds. Require five representative post-resume ingest batches at p95 under 10 seconds with no overload errors.
- The full `integrity-check` with `rank = 1` is a mandatory release gate because it detects missing rows, duplicates, and stale content that the structural/default check misses. If it cannot complete within D1's budget during rehearsal, stop before production; never substitute only the weaker check or vocabulary counts.
- If the producer cannot be stopped and the maintenance flag cannot be enforced, stop. This runbook must not improvise concurrent backfill behavior.

### Task 1: Replace the migration and Drizzle query

**Files:**
- Modify: `migrations/0003_exhibits_fts.sql`
- Create: `migrations/meta/0003_snapshot.json` so the existing journal entry has a complete base-schema snapshot
- Modify: `src/lib/api.ts`, `src/pages/api/ingest.ts`, `src/pages/search.astro`, `src/middleware.ts`, `src/pages/mcp.ts`, `src/env.d.ts`, and `wrangler.jsonc`
- Modify: `README.md`
- Test: `test/d1-read.test.ts`, `test/ingest.test.ts`, and new `test/search-maintenance.test.ts`
- Test: new `test/fts-external-schema.test.ts` for rows created before the FTS table and triggers
- Modify: `test/seed.ts` to reset the external-content vocabulary as well as the FTS and base tables

**Interfaces:**
- Consumes: existing `ex10Search(query, page, pageSize, db)`, `summaryCols`, `ftsQuery`, `orderNewest`, and Drizzle D1 binding.
- Produces: the same `SearchResult`, with `engine: 'fts'`, typed summaries, rowid-backed snippet generation, no LIKE engine or query path, and an intentional maintenance response for every caller.

**Acceptance criteria:**
- AC1: HTML search behavior remains exact-token, phrase, newest-first, paginated, and marked-snippet compatible through the public `ex10Search` surface. Measure: `pnpm vitest run test/d1-read.test.ts test/fts-external-schema.test.ts` -> all body, phrase, order, pagination, pre-trigger-row, and column-1 snippet cases pass; the old UUID join and column 2 produce explicit failures.
- AC2: real Drizzle upserts maintain FTS under D1 batching without rewriting unchanged FTS content. Measure: `pnpm vitest run test/ingest.test.ts` -> insert, changed conflict update, unchanged update, and delete cases return expected matches; D1 reports fewer `rows_written` for the unchanged update than the changed update.
- AC3: the external index matches the full base content, not just its internal structures. Measure: `pnpm vitest run test/fts-external-schema.test.ts` executes `integrity-check` with `rank = 1` after fixtures and backfill -> exit 0; deliberate missing-row, duplicate-rowid, and stale-content fixtures each fail that same check.
- AC4: maintenance mode blocks writes and never serves partial results. Measure: `pnpm vitest run test/search-maintenance.test.ts` -> ingest returns 503 with an unchanged base-row signature, HTML/markdown return 503, MCP returns `isError: true`, and normal behavior resumes when the flag is `0`.

**Stack:** branch `feat/d1-external-fts-1-schema-query` on `main`.

- [ ] **Step 0: Run AC1 through AC4 against the current schema and record RED.** Expected: the external-content, no-op write, full-content integrity, and maintenance cases are absent or fail against the standalone UUID design.
- [ ] **Step 1: Add failing tests for two pre-trigger rows, rowid joins, markdown snippets at column 1, exact counts, and the absence of LIKE.** Include explicit body-match, phrase-order, quoted-operator, pagination, and description-only cases.
- [ ] **Step 2: Add failing real-D1 tests for inserts, conflict upserts, updates that assign unchanged markdown, and deletes.** Apply the real `0000` through `0003` migration chain so Drizzle sees every schema column.
- [ ] **Step 3: Run `pnpm vitest run test/d1-read.test.ts test/ingest.test.ts`.** Expected: targeted failures against the old standalone index and UUID join.
- [ ] **Step 4: Implement the Section 1 migration and Drizzle changes.** Do not add an `id` column to the virtual table and do not remove the markdown excerpt from `summaryCols`.
- [ ] **Step 5: Add the pre-trigger fixture in `test/fts-external-schema.test.ts`.** Apply only `0000` through `0002`, insert two exhibits, then apply `0003` and the direct-rowid backfill before calling `ex10Search`. Add `integrity-check` after inserts, updates, and deletes, plus negative fixtures for missing, duplicate, and stale rows. Update `test/seed.ts` to drop `exhibits_fts_vocab`, `exhibits_fts`, and `exhibits` in that order before every reset and run two consecutive seeds in one test.
- [ ] **Step 6: Implement the maintenance flag and update-trigger guard.** Test changed and unchanged upserts in real D1 batches, paused authenticated ingest, the unchanged base-row signature, and all three caller responses. Restore the flag in `afterEach` so tests cannot leak maintenance state. Do not leave the flag permanently enabled in committed configuration.
- [ ] **Step 7: Repair migration metadata and non-production compatibility.** Create `0003_snapshot.json` from the unchanged `0002` Drizzle table snapshot with a fresh `id` and `prevId` equal to the existing `0002` snapshot's `id`; keep the journal entry and run `pnpm exec drizzle-kit check`. Recreate disposable local/staging databases, or explicitly drop their derived FTS objects and remove only their `0003` ledger row before reapplying. Never deploy rowid code against an old standalone schema and never perform that ledger deletion on production.
- [ ] **Step 8: Run all focused tests and AC1 through AC4, then `pnpm vitest run` and lint/format checks on touched files.** Record GREEN output and the unchanged-update `rows_written` comparison.
- [ ] **Step 9: Commit and add the branch with `gh stack add` / `gh stack push`.** Do not push directly to `main`.

### Task 2: Make the backfill resumable without false index-membership checks

**Files:**
- Modify: `scripts/fts-backfill.ts`
- Modify: `package.json` and `migrations/0003_exhibits_fts.sql` runner comments
- Modify: `vitest.config.ts` to include only the Worker-pool `test/**/*.test.ts` files
- Create: `vitest.node.config.ts` with `environment: 'node'` and `test-node/**/*.test.ts` inclusion
- Test: `test/fts-backfill.test.ts` for dependency-injected unit cases without child-process calls
- Test: `test-node/fts-backfill.test.ts` for the real Wrangler/child-process CLI integration fixture

**Interfaces:**
- Input: `--local` or `--remote`, optional `--env staging`, `FTS_CHUNK` as an integer from 1 to 100, a database-specific checkpoint path, an injectable Wrangler runner, and an overridable health-check origin.
- Output: one sequential chunk at a time, a durable local `{ afterRowid, pendingChunk, completed }` checkpoint, readable progress, and a nonzero exit on health-check, duration, base-signature change, or D1 failure. Store checkpoints outside the repository and back up the latest committed checkpoint.
- Test execution: dependency-inject the runner and probe in Worker-pool unit tests and do not invoke child processes there. Run the separate Node configuration with `pnpm vitest run --config vitest.node.config.ts`; it can spawn Wrangler because it does not use `cloudflareTest()`. For the CLI fixture, seed and query the same Wrangler `--persist-to <temporary-directory>` across every process; Vitest's in-process D1 is not that database.
- Invariant: no concurrent producer write, no `NOT EXISTS` against FTS, no inferred completion from `meta.changes < chunkSize`, and no blind resume after an uncertain chunk.

**Acceptance criteria:**
- AC1: more than two fixture chunks are returned in ascending rowid order and rerunning from the completed checkpoint performs no insert and adds no duplicates. Measure: `pnpm vitest run --config vitest.node.config.ts` against a persisted local D1 fixture with five pre-index rows and `FTS_CHUNK=2` -> logs `completed`; the second run logs `already complete`, and `integrity-check` with `rank = 1` still exits 0 after both runs.
- AC2: invalid configuration and unhealthy probes fail closed. Measure: run with `FTS_CHUNK=0`, `-1`, `2.5`, and a probe stub returning HTTP 500 -> each exits nonzero before issuing another chunk.
- AC3: an interrupted chunk is never silently retried into duplicate rowids. Measure: simulate an insert success followed by checkpoint-write failure -> CLI reports `RESET_REQUIRED` and refuses subsequent insertion until the specified index reset is performed.

**Stack:** branch `feat/d1-external-fts-2-backfill` on `feat/d1-external-fts-1-schema-query`.

- [ ] **Step 0: Run AC1 through AC3 and record RED or absent.** The existing Bun-only loop, 250-row default, and changes-based termination do not satisfy them.
- [ ] **Step 1: Write failing tests for positive-integer validation, empty input, exact partial final chunks, checkpoint ordering, pending-intent recovery, failed writes, failed checkpoints, base-signature changes, and bounded health-probe backoff.** Use injected runners/probes for unit tests and shared Wrangler persistence for the independent CLI integration case.
- [ ] **Step 2: Run `pnpm vitest run test/fts-backfill.test.ts`.** Expected: failures identify the missing keyset/checkpoint/health behavior.
- [ ] **Step 3: Port the script to `node:child_process.execFile` or an equivalent argument-array API.** Invoke `pnpm exec wrangler` with literal arguments, never a shell-concatenated command. Change the package runner and shebang consistently. Add the two Vitest configurations and the `test:cli` package script so Worker-pool tests never spawn processes and Node CLI tests never load the Cloudflare pool. Run `pnpm vitest run --config vitest.node.config.ts` to demonstrate the real CLI path.
- [ ] **Step 4: Implement a two-query chunk and a write-freeze assertion.** First select candidate rowids with `SELECT rowid FROM exhibits WHERE rowid > ? ORDER BY rowid LIMIT ?`. With the same `afterRowid` and chunk size, insert `exhibits.rowid`, description, and markdown directly into FTS. Complete only when candidate selection returns no rowids. Before every chunk and at completion, compare `SELECT count(*), max(rowid) FROM exhibits` with the initial snapshot; any changed signature means a straggler insert may have caused a duplicate, so enter `RESET_REQUIRED` rather than advancing.
- [ ] **Step 5: Persist `pendingChunk` before insertion, validate `meta.changes` equals the number of selected candidates, then atomically persist the largest selected rowid and clear `pendingChunk`.** Empty or partial candidate sets terminate normally. A malformed checkpoint, nonempty `pendingChunk`, invalid change count, or stopped or crashed run after insertion enters `RESET_REQUIRED` recovery rather than rerunning a possibly indexed chunk. Use write-and-rename checkpoint updates so a torn checkpoint also fails closed.
- [ ] **Step 6: Implement bounded deterministic recovery.** With both producer barriers still enabled, use FTS5 `delete-all`, reset `afterRowid` to 0, verify the vocabulary is empty, and restart from the first row. Permit one full reset per production run; a second uncertain chunk aborts for operator investigation instead of looping forever. Never claim the prior run was safely resumable before this reset.
- [ ] **Step 7: Add sequential health checks and pacing after every chunk.** Homepage and `/stats.json` must stay HTTP 200 within the 2-second p95 budget. Retry at most three times with 1-, 2-, and 4-second backoff on transient failures, then exit nonzero on resource pressure, a changed base signature, or an unhealthy probe. Do not use parallel `d1 execute` calls.
- [ ] **Step 8: Run focused Worker-pool tests and `pnpm vitest run --config vitest.node.config.ts` for the AC1 CLI fixture.** Then run `pnpm vitest run` and lint/format checks on touched files and record all GREEN measurers.
- [ ] **Step 9: Commit and add the branch with `gh stack add` / `gh stack push`.**

### Task 3: Reconcile production, roll out safely, and verify every caller

**Files:**
- Operation only after Tasks 1 and 2: remote `sec-ex10` D1 and the `sec-ex10-frontend` Worker
- Modify: `test/smoke-prod.sh` to exercise `search_agreements` using its existing JSON-RPC helper
- Modify documentation only if measured implementation details differ: `contracts_fix.md`, `README.md`

**Interfaces:**
- Consumes: the rowid-based Worker deploy, replacement migration, paused-ingest backfill, and all AC measurers from Tasks 1 and 2.
- Produces: production search with exact totals and snippets on HTML, markdown, and MCP; preserved homepage, stats, and post-resume ingest behavior.

**Acceptance criteria:**
- AC1: selective and broad searches work with exact counts and marked snippets. Measure: five sequential requests each to `/search?q=engagement`, `CapForce`, `leasing`, `credit%20agreement`, `a`, and `agreement` with a normal browser user agent -> every selective request returns HTTP 200 in under 1 second, every broad request returns HTTP 200 in under 5 seconds, and selective results are nonzero with `<mark>`.
- AC2: markdown and MCP expose actual results rather than hidden errors. Measure: `Accept: text/markdown` on `/search?q=engagement` contains matching hits; `H=https://live-contracts.arthur.law bash test/smoke-prod.sh` exits 0 and its JSON-RPC `tools/call` for `search_agreements` receives HTTP 200, `total > 0`, matching items, and no `isError: true`.
- AC3: index and base content are consistent. Measure: the backfill candidate query has no remaining rowids, the base-row signature never changed, FTS5 `integrity-check` with `rank = 1` exits 0 within 30 seconds, and vocabulary diagnostics plus sampled known inserts/updates/deletes match expected hits.
- AC4: public health, storage, and future writes stay healthy. Measure: homepage and `/stats.json` return 200 at p95 under 2 seconds throughout backfill, final `size_after` remains at or below 8 GB, and five representative post-resume conflict batches return 200 at p95 under 10 seconds without overload while stale terms disappear and new terms match.
- AC5: maintenance mode prevents partial reads and writes. Measure: while enabled, authenticated `rows: []` ingest returns 503, two base-row signatures 60 seconds apart are identical, and HTML/markdown return 503 while MCP reports a maintenance `isError: true`; no partial search result is served.

**Stack:** branch `feat/d1-external-fts-3-rollout` on `feat/d1-external-fts-2-backfill`.

- [ ] **Step 0: Record all current RED baselines and the exact pre-change Worker deployment ID.** Run AC1 and AC2 and record the HTML 500, markdown fallback behavior, and MCP `isError` separately. Record AC3 as `INDEX_ABSENT`, AC4's current health/storage plus the unavailable post-resume check, and AC5 as `MAINTENANCE_FLAG_ABSENT`. Use the recorded deployment, previously observed as `1b3c46b6` only if still current, as the rollback target.
- [ ] **Step 1: Re-run read-only diagnosis before any write.** Query `d1_migrations`, `sqlite_master.sql`, and, if supported remotely, the two PRAGMAs. Record exhibit count, `max(rowid)`, database size, Time Travel checkpoint, and absence of FTS objects. Stop if production contains `0003` or any FTS object.
- [ ] **Step 2: Normalize the migration ledger, then reconcile `0002`.** Wrangler compares complete filenames including `.sql`. If the legacy ledger literally contains `0000_flaky_avengers` and `0001_bumpy_tusk`, first verify that neither filename-form row also exists, then update those two names byte-exactly to `0000_flaky_avengers.sql` and `0001_bumpy_tusk.sql`; otherwise Wrangler will offer already-applied `0000` and `0001`. If all three `0002` columns and the correct index exist, insert only `0002_boring_union_jack.sql`; otherwise apply only missing DDL before that insert. Compare the directory with the resulting ledger and record that only `0003` remains. Decline the Step 6 interactive prompt if it offers anything else.
- [ ] **Step 3: Pass the production-scale rehearsal against every numeric gate.** Prove chunk and health latency, the at-most-2-hour pause estimate, final storage at or below 8 GB, bounded merge behavior, full `integrity-check` with `rank = 1` within 30 seconds, selective/broad query latency, and post-resume ingest with unchanged re-pushes. Stop before production if any gate fails; there is no weaker-verification fallback.
- [ ] **Step 4: Freeze all other deployments and deploy Task 1 before migrating, with maintenance disabled.** Pause the deployment pipeline and prohibit other operators/branches from deploying through Step 13. Run `pnpm run build`, then `pnpm exec wrangler deploy --keep-vars --var SEARCH_MAINTENANCE:0` from `frontend/`. Search may retain the honest missing-table 500 temporarily. Confirm the rowid join, snippet column 1, maintenance handler, committed default `"0"`, and absent LIKE fallback.
- [ ] **Step 5: Stop the external producer, then enforce maintenance mode.** Deploy with `pnpm exec wrangler deploy --keep-vars --var SEARCH_MAINTENANCE:1`. An authenticated request with `rows: []` must return 503, and two `SELECT count(*), max(rowid) FROM exhibits` snapshots taken 60 seconds apart must match exactly. Verify HTML/markdown 503, MCP maintenance `isError: true`, and healthy homepage/stats. Do not proceed if any write lands.
- [ ] **Step 6: Apply only replacement `0003` interactively.** Verify `content='exhibits'`, `content_rowid='rowid'`, only description/markdown FTS columns, the guarded update trigger, all three triggers, and the vocabulary table. Do not expect `SELECT count(*) FROM exhibits_fts` to be zero; verify initial index emptiness with `SELECT doc FROM exhibits_fts_vocab LIMIT 1`, which must return zero rows.
- [ ] **Step 7: Run the Task 2 backfill at chunk size 10.** Reduce to 1 on resource pressure and increase only while the fixed duration, health, and 2-hour pause budgets hold. Check base signature, database size, homepage, stats, and D1 errors between chunks. Keep both producer barriers enabled and permit at most one full reset.
- [ ] **Step 8: Prove completion before serving results.** Exhaust candidates with no pending checkpoint or base-signature change. Apply `INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('merge', 500)` in bounded batches only as rehearsed, record final `size_after <= 8 GB`, then run `integrity-check` with `rank = 1` within 30 seconds. Failure leaves maintenance enabled and the producer stopped. Vocabulary counts require zero-token accounting and never replace this check.
- [ ] **Step 9: Verify direct FTS behavior while maintenance remains enabled.** Run `MATCH` counts for `engagement` and `agreement`, selective snippets, and sampled updated/deleted fixture terms. Keep `summaryCols` intact and confirm that no caller can see partial results.
- [ ] **Step 10: Re-enable reads before restarting the external producer.** Deploy with `SEARCH_MAINTENANCE:0` while the producer remains stopped, then run AC1 and AC2. If either fails, restore `SEARCH_MAINTENANCE:1`; do not start the producer.
- [ ] **Step 11: Use the explicit rollback path on any abort.** Keep the external producer stopped. For an index-only failure, keep the Task 1 Worker in maintenance and use the bounded Task 2 reset. For an unrecoverable state, Time Travel restore first while the Task 1 Worker still returns intentional 503s, then verify the restored ledger/schema. To resume the pre-change system, deploy the exact pre-change Worker ID recorded in Step 0 and replay the producer's retained backlog only after confirming that the restored database has no FTS objects. This restores ingest and the original broken-search behavior, not search. Never select an intermediate version whose catch could activate LIKE and never roll back only one side of the schema/code pair. Resume other deployments only after the compatible pair is verified.
- [ ] **Step 12: Start the external producer gradually and run AC4.** Watch actual changed and unchanged conflict re-pushes through `db.batch`, confirm the null-safe trigger avoids unnecessary FTS rewrites, and enforce the 10-second p95/no-overload budget during backlog replay. Re-pause the external producer if that budget fails.
- [ ] **Step 13: Record final storage, latencies, all AC outputs, rollback state, and the recovery checkpoint.** Update this plan and README with measured facts only. Commit and use `gh stack add` / `gh stack push`; CI/CD is required on this last task. Publish and verify that final deployment with `SEARCH_MAINTENANCE:0` before unfreezing other deployments.

---

## 3. Retained analysis and compatible original notes

### Original root cause remains valid

`/search` failed because production never received `exhibits_fts`. The same `ex10Search` function serves:

- `src/pages/search.astro` for nonempty HTML searches.
- `src/middleware.ts` for `Accept: text/markdown` searches.
- `src/pages/mcp.ts` tool `search_agreements`.

Their failure surfaces differ. `search.astro` can render an HTTP 500. The markdown middleware catches the data error and falls through to HTML. MCP catches the error and normally returns HTTP 200 with a JSON-RPC result marked `isError: true`.

The original direct query error was `no such table: exhibits_fts` (`SQLITE_ERROR` 7500), and remote `d1_migrations` contained only `0000_flaky_avengers` and `0001_bumpy_tusk`. Recheck these observations before production work.

The Workers log showed the FTS count failing for `engagement`, `CapForce`, `leasing`, `credit agreement`, and `a`. Wall time was about 100 ms because the statement failed immediately, not because search was slow or term-specific.

`DrizzleQueryError` puts `Failed query: ...` in `message` and the SQLite error in `cause`. The old catch tested only `String(e)`, so it always rethrew. This was fortunate because otherwise it could enable the dangerous fallback. Deleting the fallback is required; adding a cause-aware predicate that enables it is forbidden.

### `ftsQuery` remains usable, with corrected wording

`AND`, `OR`, and `NOT` become quoted search terms rather than user-supplied boolean operators. `*`, `^`, `:`, and parentheses are removed by `src/lib/summary.ts` before remaining tokens are quoted; they are not searched as literal punctuation. Quoted phrase order remains meaningful.

Local Vitest applies every migration in `test/seed.ts`, so passing tests never proved that production was migrated. Task 1 now also tests the real Drizzle batch path and external-content rowid semantics.

### The ingest overload remains a separate incident

| | Search failure | Ingest issue |
|---|---|---|
| Route | `GET /search` | `POST /api/ingest` |
| Error | `Failed query: select count(*) from exhibits_fts ...` | `D1 DB is overloaded. Requests queued for too long` |
| Previously observed Worker | `1b3c46b6` | `5c6fd9d7`, older |
| Code path | `ex10Search` count | `db.batch` in `src/pages/api/ingest.ts` |
| Trigger | Missing table on a read | Producer batch rejected before upsert |

The previously observed ingest trace died in 3 ms after `d1_batch`; the upsert did not run. Retrying ingest could not create the FTS table. The replacement plan's deliberate producer pause does not change that diagnosis; it prevents new upserts from firing external-content triggers against unbackfilled rows.

The homepage already uses bare `ORDER BY filed_at` with `idx_ex_filed_at`. Do not restore `IS NULL`, `found_at`, or `id` sort expressions, and do not recreate `idx_ex_filed_newest`.

### Why request-time full-text indexing is still required

The corpus is approximately 19,554 exhibits and 1.8 GB, mostly markdown. `description LIKE ... OR markdown LIKE ...` can scan that content and block the single-writer D1 database. The page and MCP promise live request-time search as new filings arrive, so these alternatives still do not fit:

- Pagefind and other build-time Astro indexes.
- Astro content collections backed by files.
- Client-side Fuse, MiniSearch, or full-corpus downloads.
- The old `LIKE` fallback.

External-content FTS remains one search system in D1. It reduces storage duplication but still needs token, position, and document-size storage and measured headroom.

### Search field boundaries remain intentional

The FTS table indexes only description and markdown. Company name is still parsed from `filing_metadata`; CIK, accession, and form type remain structured columns. Contract party names found in markdown are searchable. Filer names found only in JSON are outside this fix. Do not index raw `filing_metadata` JSON.

Description-only matches still return their typed exhibit rows because both columns are indexed. Markdown is snippet column 1 in the replacement schema.

### Original migration-collision warning remains applicable

`pnpm db:migrate` applies every unrecorded migration in order and compares complete filenames including `.sql`. The earlier short-name ledger observation may reflect legacy/manual rows, so normalize those names byte-exactly before applying migrations. Reconcile `0002_boring_union_jack.sql` before applying `0003`, because its non-idempotent statements are:

```sql
ALTER TABLE exhibits ADD source text;
ALTER TABLE exhibits ADD size_bytes integer;
ALTER TABLE exhibits ADD detected_at text;
CREATE INDEX idx_ex_detected_at ON exhibits (detected_at);
```

An existing column makes that migration fail before `0003` runs. Inspect object definitions rather than inferring them from successful ingest or the missing ledger row.

The ledger and `sqlite_master` probes are required. Run the PRAGMAs only after confirming that remote D1 accepts them:

```bash
pnpm exec wrangler d1 execute sec-ex10 --remote --command "SELECT id, name FROM d1_migrations ORDER BY id"
pnpm exec wrangler d1 execute sec-ex10 --remote --command "PRAGMA table_info(exhibits)"
pnpm exec wrangler d1 execute sec-ex10 --remote --command "SELECT name, type, sql FROM sqlite_master WHERE type IN ('table','index','trigger') ORDER BY name"
pnpm exec wrangler d1 execute sec-ex10 --remote --command "PRAGMA index_info(idx_ex_detected_at)"
```

Do not select full markdown or run LIKE against it during diagnosis.

### Broad-query and rendering cautions remain applicable

A broad token can still produce a large FTS hit list and sort it by `filed_at`; `idx_ex_filed_at` does not eliminate that FTS-hit sort. Measure `a` and `agreement` rather than declaring success only on selective terms.

`DocketRow` uses the FTS snippet for the card body but still calls `displayTitle(item.description, item.excerpt_md || item.excerpt)`. Removing `substr(markdown, 1, 2000)` would degrade label-only document titles. Any future query-shape optimization must preserve title data.

### Out of scope

- Rewriting stored markdown or existing render-time emphasis handling.
- Indexing raw `filing_metadata` JSON.
- Changing the homepage `ORDER BY`.
- Diagnosing the older ingest overload's original root cause beyond preventing unnecessary FTS rewrites, producer pause, catch-up pacing, and overload abort checks.
- AI Search, Vectorize, Pagefind, client-side bundles, or an external search vendor.
