/**
 * FTS5 backfill core: index the `exhibits` rows that predate the 0003 triggers.
 *
 * Pure logic with injected I/O so it runs under Vitest without child processes.
 * The CLI wrapper (fts-backfill.ts) supplies `exec` through `wrangler d1 execute`.
 *
 * Safety model (contracts_fix.md §4):
 *  - the external producer is stopped; the base signature `(count, max rowid)` is
 *    snapshotted and re-checked before every chunk and at the end;
 *  - chunks are fixed rowid ranges (lo, hi], applied one at a time;
 *  - no checkpoint file: a resume from `--from N` is accepted only when the index
 *    holds exactly the rows with rowid <= N, which also detects a chunk that
 *    committed but was never logged;
 *  - after every chunk the index must hold exactly the rows with rowid <= hi, which
 *    catches a trigger that indexed a row ahead of the backfill (an UPDATE leaves the
 *    signature unchanged);
 *  - every successful run ends with `integrity-check`, the only check that compares
 *    the index against the content table row by row;
 *  - recovery from any doubt is `delete-all` and a fresh run from 0.
 */

export interface Opts {
  mode: 'local' | 'remote';
  db: string;
  from: number;
  to?: number;
  chunk: number;
  origin?: string;
  persistTo?: string;
}

export interface ExecResult {
  rows: Record<string, unknown>[];
  /** Undefined when wrangler omits `meta.changes` (local `--json` reports only `duration`). */
  changes?: number;
  /** Undefined when wrangler omits `meta.size_after`. */
  sizeAfter?: number;
  durationMs: number;
}

export interface Deps {
  exec(sql: string): Promise<ExecResult>;
  probe(url: string): Promise<number>;
  log(line: string): void;
  sleep(ms: number): Promise<void>;
}

/**
 * An error D1 itself reported (wrangler's structured error JSON). Only these can
 * prove the index corrupt; stderr or parser text never can (#89).
 */
export class D1Error extends Error {
  override name = 'D1Error';
}

export const EXIT = {
  DONE: 0,
  SIGNATURE_CHANGED: 2,
  UNHEALTHY: 3,
  CHUNK_FAILED: 4,
  SIZE_CAP: 5,
  COUNT_MISMATCH: 6,
  INDEX_STATE: 7,
  INTEGRITY: 8,
  EXEC_FAILED: 9,
  USAGE: 64,
} as const;

const SIZE_CAP_BYTES = 8_000_000_000;
const RETRY_DELAYS_MS = [1000, 2000, 4000];
const DEFAULT_ORIGIN = 'https://live-contracts.arthur.law';
const RESET = "INSERT INTO exhibits_fts(exhibits_fts) VALUES('delete-all')";
const INTEGRITY_CHECK = "INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', 1)";

const usage = (why: string) =>
  new Error(
    `usage: ${why}\n  fts-backfill (--remote --to <max rowid> | --local [--persist-to <dir>]) [--from N] [--chunk 1..500] [--db name] [--origin url]`,
  );

function int(name: string, raw: string | undefined, min: number, max: number): number {
  if (raw === undefined || !/^-?\d+$/.test(raw)) throw usage(`${name} must be an integer`);
  const n = Number(raw);
  if (n < min || n > max) throw usage(`${name} must be between ${min} and ${max}`);
  return n;
}

/** Parse CLI flags. Throws a `usage:` error on anything unknown or out of range. */
export function parseArgs(argv: string[]): Opts {
  let mode: Opts['mode'] | undefined;
  const o: Partial<Opts> = { db: 'sec-ex10', from: 0, chunk: 50 };
  let origin: string | undefined;
  const seen = new Set<string>();
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => argv[++i];
    if (a !== '--remote' && a !== '--local') {
      if (seen.has(a)) throw usage(`${a} given twice`);
      seen.add(a);
    }
    if (a === '--remote' || a === '--local') {
      const m = a === '--remote' ? 'remote' : 'local';
      if (mode) throw usage(mode === m ? `${a} given twice` : 'choose one of --remote or --local');
      mode = m;
    } else if (a === '--to') o.to = int('--to', val(), 0, Number.MAX_SAFE_INTEGER);
    else if (a === '--from') o.from = int('--from', val(), 0, Number.MAX_SAFE_INTEGER);
    else if (a === '--chunk') o.chunk = int('--chunk', val(), 1, 500);
    else if (a === '--db') o.db = val();
    else if (a === '--origin') origin = val();
    else if (a === '--persist-to') o.persistTo = val();
    else throw usage(`unknown argument ${a}`);
  }
  if (!mode) throw usage('--remote or --local is required');
  if (mode === 'local' && origin !== undefined) throw usage('--local runs skip the health probe; drop --origin');
  if (mode === 'remote' && o.to === undefined)
    throw usage('--remote needs --to <max rowid> taken after the producer stopped');
  if (o.to !== undefined && o.from! > o.to) throw usage('--from must not exceed --to');
  if (!o.db) throw usage('--db needs a name');
  return { ...(o as Opts), to: o.to, mode, origin: mode === 'remote' ? (origin ?? DEFAULT_ORIGIN) : undefined };
}

/** Half-open rowid ranges (lo, hi] covering (from, to] in ascending order. */
export function planRanges(from: number, to: number, chunk: number): [number, number][] {
  const out: [number, number][] = [];
  for (let lo = from; lo < to; lo += chunk) out.push([lo, Math.min(lo + chunk, to)]);
  return out;
}

export const chunkSql = (lo: number, hi: number) =>
  `INSERT INTO exhibits_fts(rowid, description, markdown) SELECT rowid, coalesce(description, ''), coalesce(markdown, '') FROM exhibits WHERE rowid > ${lo} AND rowid <= ${hi}`;

const num = (r: ExecResult, k: string) => Number(r.rows[0]?.[k] ?? 0);

/** Run the backfill. Returns the exit code and the last rowid known to be indexed. */
export async function runBackfill(deps: Deps, opts: Opts): Promise<{ code: number; lastHi: number }> {
  let lastHi = opts.from;
  try {
    return await backfill(deps, opts, (hi) => (lastHi = hi));
  } catch (e) {
    deps.log(`EXEC_FAILED: ${(e as Error).message}`);
    deps.log(
      `Stopped on an error. If D1 or wrangler failed, rerun with --from ${lastHi}; the resume check refuses if the index moved. Otherwise fix the cause first.`,
    );
    return { code: EXIT.EXEC_FAILED, lastHi };
  }
}

async function backfill(
  deps: Deps,
  opts: Opts,
  advance: (hi: number) => void,
): Promise<{ code: number; lastHi: number }> {
  const log = (line: string) => deps.log(line);
  const signature = async () => {
    const r = await deps.exec('SELECT count(*) AS n, max(rowid) AS m FROM exhibits');
    return { n: num(r, 'n'), m: num(r, 'm') };
  };
  const docs = async () => num(await deps.exec('SELECT count(*) AS n FROM exhibits_fts_docsize'), 'n');
  const rowsAtOrBelow = async (id: number) =>
    num(await deps.exec(`SELECT count(*) AS n FROM exhibits WHERE rowid <= ${id}`), 'n');
  const changed = (a: { n: number; m: number }, b: { n: number; m: number }) => a.n !== b.n || a.m !== b.m;
  const stopSignature = (lastHi: number) => {
    log(`SIGNATURE_CHANGED: producer wrote during backfill. Run ${RESET} and restart from 0.`);
    return { code: EXIT.SIGNATURE_CHANGED, lastHi };
  };

  const base = await signature();
  const to = opts.to ?? base.m;
  if (base.m !== to) {
    log(`max(rowid) is ${base.m}, expected --to ${to}: stop the producer and retake the snapshot.`);
    return stopSignature(opts.from);
  }

  const indexed = await docs();
  const expected = opts.from === 0 ? 0 : await rowsAtOrBelow(opts.from);
  if (indexed !== expected) {
    log(
      `INDEX_STATE: index holds ${indexed} documents, expected ${expected} for --from ${opts.from}. Run ${RESET} and restart from 0.`,
    );
    return { code: EXIT.INDEX_STATE, lastHi: opts.from };
  }

  let lastHi = opts.from;
  for (const [lo, hi] of planRanges(opts.from, to, opts.chunk)) {
    if (changed(await signature(), base)) return stopSignature(lastHi);
    let r: ExecResult;
    try {
      r = await deps.exec(chunkSql(lo, hi));
    } catch (e) {
      log(`CHUNK_FAILED (${lo}, ${hi}]: ${(e as Error).message}`);
      log(
        `If the statement rolled back, rerun with --from ${lastHi}. If it committed before the response was lost, that rerun refuses with INDEX_STATE (exit 7): run ${RESET} and restart from 0.`,
      );
      return { code: EXIT.CHUNK_FAILED, lastHi };
    }
    lastHi = hi;
    advance(hi);
    const size = r.sizeAfter === undefined ? 'n/a' : String(r.sizeAfter);
    log(`chunk (${lo}, ${hi}] changes=${r.changes ?? 'n/a'} ms=${r.durationMs} size=${size}`);
    if (r.sizeAfter === undefined && opts.mode === 'remote') {
      log('SIZE_CAP: D1 did not report size_after, so the 8 GB abort line cannot be enforced. Stopping.');
      return { code: EXIT.SIZE_CAP, lastHi };
    }
    if (r.sizeAfter !== undefined && r.sizeAfter > SIZE_CAP_BYTES) {
      log(`SIZE_CAP: database is ${r.sizeAfter} bytes, above the 8 GB abort line.`);
      return { code: EXIT.SIZE_CAP, lastHi };
    }
    const [have, want] = [await docs(), await rowsAtOrBelow(hi)];
    if (have !== want) {
      log(
        `INDEX_STATE: after chunk (${lo}, ${hi}] the index holds ${have} documents, expected ${want}. A trigger indexed rows ahead of the backfill. Run ${RESET} and restart from 0.`,
      );
      return { code: EXIT.INDEX_STATE, lastHi };
    }
    if (opts.origin && !(await healthy(deps, `${opts.origin}/stats.json`))) {
      log(`UNHEALTHY: ${opts.origin}/stats.json kept failing. Resume later with --from ${lastHi}.`);
      return { code: EXIT.UNHEALTHY, lastHi };
    }
  }

  if (changed(await signature(), base)) return stopSignature(lastHi);
  const finalDocs = await docs();
  if (finalDocs !== base.n) {
    log(`COUNT_MISMATCH: index holds ${finalDocs} documents for ${base.n} rows. Run ${RESET} and restart from 0.`);
    return { code: EXIT.COUNT_MISMATCH, lastHi };
  }
  try {
    await deps.exec(INTEGRITY_CHECK);
  } catch (e) {
    const msg = (e as Error).message;
    // Only a rejection of the index is INTEGRITY. A CPU limit or a lost response
    // says nothing about the index, and delete-all would wipe a healthy one.
    // SQLite's own SQLITE_CORRUPT text in D1's structured error (#88, #89).
    if (e instanceof D1Error && /database disk image is malformed/i.test(msg)) {
      log(`INTEGRITY: integrity-check failed: ${msg}. Run ${RESET} and restart from 0.`);
      return { code: EXIT.INTEGRITY, lastHi };
    }
    log(
      `EXEC_FAILED: integrity-check did not finish: ${msg}. The index was not judged; rerun ${INTEGRITY_CHECK} alone.`,
    );
    return { code: EXIT.EXEC_FAILED, lastHi };
  }
  log(`DONE to=${to}`);
  return { code: EXIT.DONE, lastHi };
}

async function healthy(deps: Deps, url: string): Promise<boolean> {
  for (let attempt = 0; ; attempt++) {
    let status = 0;
    try {
      status = await deps.probe(url);
    } catch {
      status = 0;
    }
    if (status === 200) return true;
    if (attempt >= RETRY_DELAYS_MS.length) return false;
    await deps.sleep(RETRY_DELAYS_MS[attempt]);
  }
}

/**
 * Parse `wrangler d1 execute --json` stdout into one statement's result. The JSON
 * array starts at the first line that begins with `[`; anything before it (remote
 * banners, pnpm notices) is ignored. `changes` and `sizeAfter` stay undefined when
 * wrangler omits them, so callers can fail closed instead of reading a fake 0.
 */
export function parseWranglerJson(stdout: string): ExecResult {
  let parsed: unknown[] | undefined;
  for (const v of jsonCandidates(stdout, '[')) {
    if (Array.isArray(v)) {
      parsed = v;
      break;
    }
  }
  if (!parsed) throw new Error(`wrangler printed no JSON array: ${stdout.trim().slice(0, 200)}`);
  if (parsed.length === 0) throw new Error('wrangler returned an empty result array');
  const first = (parsed[0] ?? {}) as Record<string, unknown>;
  if (first.success !== true) throw new Error(`wrangler reported success=${String(first.success)}`);
  const meta = (first.meta ?? {}) as Record<string, unknown>;
  const opt = (v: unknown) => (v === undefined || v === null ? undefined : Number(v));
  return {
    rows: (first.results ?? []) as Record<string, unknown>[],
    changes: opt(meta.changes),
    sizeAfter: opt(meta.size_after),
    durationMs: Number(meta.duration ?? 0),
  };
}

/**
 * Parsed JSON values that start at a line-initial `opener` in `text`, in order.
 * Each candidate is matched with one string-aware bracket scan, so banners (even
 * ones starting with the opener) and trailing lines are skipped. An unclosed
 * candidate scans to the end, so the total scan is budgeted (#87, #88, #89).
 */
function* jsonCandidates(text: string, opener: '[' | '{'): Generator<unknown> {
  let budget = SCAN_BUDGET * (text.length + 1);
  for (const m of text.matchAll(opener === '[' ? /^\[/gm : /^\{/gm)) {
    const end = matchingBracket(text, m.index);
    budget -= (end < 0 ? text.length : end) - m.index;
    if (budget < 0) return;
    if (end < 0) continue;
    try {
      yield JSON.parse(text.slice(m.index, end + 1)) as unknown;
    } catch {
      // not JSON; try the next candidate
    }
  }
}

/**
 * Total scan allowed, in lengths of the text, shared by every candidate. An
 * unclosed line-initial `[` or `{` scans from where it starts to the end, so a few
 * such lines near the top fit and many give up (pinned in tests). Real wrangler
 * output has no column-0 junk brackets, so it never reaches the limit.
 */
const SCAN_BUDGET = 4;

/** Index of the bracket closing the one at `start` (`[` or `{`), skipping JSON strings; -1 if none. */
function matchingBracket(text: string, start: number): number {
  const open = text[start];
  const close = open === '{' ? '}' : ']';
  let depth = 0;
  let inString = false;
  for (let i = start; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      if (c === '\\') i++;
      else if (c === '"') inString = false;
    } else if (c === '"') inString = true;
    else if (c === open) depth++;
    else if (c === close && --depth === 0) return i;
  }
  return -1;
}

/**
 * Map a failed `wrangler d1 execute --json` run to an Error. D1's structured error
 * JSON on stdout (`{"error": {"text", "notes"}}`), when present, becomes a D1Error,
 * whatever banner or trailing line surrounds it. wrangler's own failures (auth,
 * config) print text on stderr and become a plain Error, as does anything else.
 * INTEGRITY needs a D1Error *and* SQLite's SQLITE_CORRUPT phrase (see runBackfill),
 * so the tag alone never triggers delete-all advice. Only non-empty string
 * `text`/`notes` values count; a candidate without any is skipped, and if none
 * qualifies the mapping falls back to trimmed stderr, or to the caller's `message`
 * when that is blank.
 */
export function execFailure(stdout: string | undefined, stderr: string | undefined, message: string): Error {
  for (const v of jsonCandidates(stdout ?? '', '{')) {
    const e = (v as { error?: { text?: unknown; notes?: unknown } } | null)?.error;
    const notes = Array.isArray(e?.notes) ? (e.notes as { text?: unknown }[]).map((n) => n?.text) : [];
    const text = [e?.text, ...notes].filter((t): t is string => typeof t === 'string' && t !== '').join(': ');
    if (text) return new D1Error(text);
  }
  return new Error(stderr?.trim() || message);
}
