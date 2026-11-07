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
  changes: number;
  sizeAfter: number;
  durationMs: number;
}

export interface Deps {
  exec(sql: string): Promise<ExecResult>;
  probe(url: string): Promise<number>;
  log(line: string): void;
  sleep(ms: number): Promise<void>;
}

export const EXIT = {
  DONE: 0,
  SIGNATURE_CHANGED: 2,
  UNHEALTHY: 3,
  CHUNK_FAILED: 4,
  SIZE_CAP: 5,
  COUNT_MISMATCH: 6,
  INDEX_STATE: 7,
  USAGE: 64,
} as const;

const SIZE_CAP_BYTES = 8_000_000_000;
const RETRY_DELAYS_MS = [1000, 2000, 4000];
const DEFAULT_ORIGIN = 'https://live-contracts.arthur.law';
const RESET = "INSERT INTO exhibits_fts(exhibits_fts) VALUES('delete-all')";

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
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => argv[++i];
    if (a === '--remote' || a === '--local') {
      const m = a === '--remote' ? 'remote' : 'local';
      if (mode && mode !== m) throw usage('choose one of --remote or --local');
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
  if (mode === 'remote' && o.to === undefined)
    throw usage('--remote needs --to <max rowid> taken after the producer stopped');
  if (o.to !== undefined && o.from! > o.to) throw usage('--from must not exceed --to');
  if (!o.db) throw usage('--db needs a name');
  return { ...(o as Opts), to: o.to, mode, origin: mode === 'remote' ? (origin ?? DEFAULT_ORIGIN) : origin };
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
  const log = (line: string) => deps.log(line);
  const signature = async () => {
    const r = await deps.exec('SELECT count(*) AS n, max(rowid) AS m FROM exhibits');
    return { n: num(r, 'n'), m: num(r, 'm') };
  };
  const docs = async () => num(await deps.exec('SELECT count(*) AS n FROM exhibits_fts_docsize'), 'n');
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
  const belowSql = `SELECT count(*) AS n FROM exhibits WHERE rowid <= ${opts.from}`;
  const expected = opts.from === 0 ? 0 : num(await deps.exec(belowSql), 'n');
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
        `A failed D1 statement rolls back. Rerun with --from ${lastHi}; the resume check refuses if the chunk did commit.`,
      );
      return { code: EXIT.CHUNK_FAILED, lastHi };
    }
    lastHi = hi;
    log(`chunk (${lo}, ${hi}] changes=${r.changes} ms=${r.durationMs} size=${r.sizeAfter}`);
    if (r.sizeAfter > SIZE_CAP_BYTES) {
      log(`SIZE_CAP: database is ${r.sizeAfter} bytes, above the 8 GB abort line.`);
      return { code: EXIT.SIZE_CAP, lastHi };
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
