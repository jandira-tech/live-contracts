// Backfill core (AC 2.1–2.5 in contracts_fix.md §4). Pure functions with injected
// deps: no child processes run inside the Workers pool.
import { describe, it, expect } from 'vitest';
import {
  D1Error,
  EXIT,
  execFailure,
  parseArgs,
  parseWranglerJson,
  planRanges,
  runBackfill,
  type Deps,
  type ExecResult,
} from '../scripts/fts-backfill-core';

describe('planRanges (AC 2.1)', () => {
  it('covers (from, to] in ascending half-open chunks', () => {
    expect(planRanges(0, 7, 3)).toEqual([
      [0, 3],
      [3, 6],
      [6, 7],
    ]);
    expect(planRanges(6, 7, 3)).toEqual([[6, 7]]);
    expect(planRanges(7, 7, 3)).toEqual([]);
    expect(planRanges(0, 100, 50)).toEqual([
      [0, 50],
      [50, 100],
    ]);
  });
});

describe('parseArgs (AC 2.2)', () => {
  it('accepts a remote run with an explicit --to', () => {
    expect(parseArgs(['--remote', '--to', '19554'])).toMatchObject({
      mode: 'remote',
      db: 'sec-ex10',
      from: 0,
      to: 19554,
      chunk: 50,
      origin: 'https://live-contracts.arthur.law',
    });
  });
  it('local runs skip the health probe and may omit --to', () => {
    expect(parseArgs(['--local', '--persist-to', '/tmp/x', '--chunk', '3'])).toMatchObject({
      mode: 'local',
      chunk: 3,
      persistTo: '/tmp/x',
      origin: undefined,
      to: undefined,
    });
  });
  it.each([
    [['--remote', '--to', '9', '--chunk', '0']],
    [['--remote', '--to', '9', '--chunk', '-1']],
    [['--remote', '--to', '9', '--chunk', '2.5']],
    [['--remote', '--to', '9', '--chunk', '501']],
    [['--remote']],
    [['--to', '9']],
    [['--local', '--remote', '--to', '9']],
    [['--remote', '--to', '9', '--from', '10']],
    [['--remote', '--to', '9', '--env', 'staging']],
    [['--local', '--origin', 'https://live-contracts.arthur.law']],
    [['--remote', '--to', '9', '--from', '1', '--from', '2']],
    [['--remote', '--remote', '--to', '9']],
  ])('rejects %j', (argv) => {
    expect(() => parseArgs(argv)).toThrow(/usage/i);
  });
});

/** Rowids 1..n. */
const ids = (n: number) => Array.from({ length: n }, (_, i) => i + 1);

/**
 * A scripted D1 over a row-set model: `rows` are the rowids in `exhibits`, `index`
 * the rowids in the FTS index (duplicates allowed, as FTS5 allows them).
 * `size: null` models wrangler omitting `meta.size_after`.
 */
function fakeD1(
  state: { rows: number[]; index: number[]; size?: number | null; corrupt?: boolean; integrityError?: string },
  hooks: { onChunk?: (i: number) => void; failChunk?: number; onRead?: (sql: string) => void } = {},
) {
  const writes: string[] = [];
  let integrityChecks = 0;
  const exec = async (sql: string): Promise<ExecResult> => {
    const r = (rows: Record<string, unknown>[], changes = 0): ExecResult => ({
      rows,
      changes,
      sizeAfter: state.size === null ? undefined : (state.size ?? 1000),
      durationMs: 5,
    });
    if (sql.startsWith('SELECT')) hooks.onRead?.(sql);
    if (/^SELECT count\(\*\) AS n, max\(rowid\) AS m FROM exhibits$/.test(sql))
      return r([{ n: state.rows.length, m: Math.max(0, ...state.rows) }]);
    if (/^SELECT count\(\*\) AS n FROM exhibits_fts_docsize$/.test(sql)) return r([{ n: state.index.length }]);
    const below = sql.match(/^SELECT count\(\*\) AS n FROM exhibits WHERE rowid <= (\d+)$/);
    if (below) return r([{ n: state.rows.filter((id) => id <= Number(below[1])).length }]);
    if (sql === "INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', 1)") {
      integrityChecks++;
      if (state.integrityError) throw new Error(state.integrityError);
      if (state.corrupt) throw new D1Error('D1_ERROR: database disk image is malformed');
      return r([]);
    }
    const chunk = sql.match(
      /^INSERT INTO exhibits_fts\(rowid, description, markdown\) SELECT rowid, coalesce\(description, ''\), coalesce\(markdown, ''\) FROM exhibits WHERE rowid > (\d+) AND rowid <= (\d+)$/,
    );
    if (chunk) {
      const i = writes.length;
      writes.push(sql);
      if (hooks.failChunk === i) throw new Error('D1_ERROR: boom');
      const [lo, hi] = [Number(chunk[1]), Number(chunk[2])];
      const added = state.rows.filter((id) => id > lo && id <= hi);
      state.index.push(...added);
      hooks.onChunk?.(i);
      return r([], added.length);
    }
    throw new Error(`unexpected SQL: ${sql}`);
  };
  return { exec, writes, integrityChecks: () => integrityChecks };
}
const deps = (exec: Deps['exec'], probe: Deps['probe'] = async () => 200) => {
  const lines: string[] = [];
  const probes: string[] = [];
  const d: Deps = {
    exec,
    probe: async (u) => (probes.push(u), probe(u)),
    log: (l) => lines.push(l),
    sleep: async () => {},
  };
  return { d, lines, probes };
};
const BASE = { mode: 'remote' as const, db: 'sec-ex10', from: 0, chunk: 3, origin: 'https://o' };

describe('runBackfill', () => {
  it('indexes every range, runs integrity-check, and ends with DONE (AC 2.5 from 0)', async () => {
    const f = fakeD1({ rows: ids(7), index: [] });
    const { d, lines, probes } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.DONE, lastHi: 7 });
    expect(f.writes).toHaveLength(3);
    expect(f.writes[0]).toMatch(/WHERE rowid > 0 AND rowid <= 3$/);
    expect(f.writes[0]).not.toMatch(/NOT EXISTS/);
    expect(probes).toEqual(['https://o/stats.json', 'https://o/stats.json', 'https://o/stats.json']);
    expect(f.integrityChecks()).toBe(1);
    expect(lines.at(-1)).toBe('DONE to=7');
  });

  it('handles rowid gaps: 6 rows with max rowid 7 index to 6 documents', async () => {
    const f = fakeD1({ rows: [1, 2, 3, 4, 5, 7], index: [] });
    const { d } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.DONE, lastHi: 7 });
  });

  it('resumes from --from when the index holds exactly the rows at or below it (AC 2.5)', async () => {
    const f = fakeD1({ rows: ids(7), index: ids(6) });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, from: 6, to: 7 })).toEqual({ code: EXIT.DONE, lastHi: 7 });
    expect(f.writes).toHaveLength(1);
    expect(f.writes[0]).toMatch(/rowid > 6 AND rowid <= 7$/);
    expect(lines.at(-1)).toBe('DONE to=7');
  });

  it('refuses a fresh run over a non-empty index', async () => {
    const f = fakeD1({ rows: ids(7), index: [1, 2] });
    const { d, lines } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.INDEX_STATE);
    expect(f.writes).toHaveLength(0);
    expect(lines.join('\n')).toMatch(/delete-all/);
  });

  it('refuses to resume when an unlogged chunk already committed', async () => {
    const f = fakeD1({ rows: ids(7), index: ids(6) });
    const { d } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, from: 3, to: 7 })).code).toBe(EXIT.INDEX_STATE);
    expect(f.writes).toHaveLength(0);
  });

  it('refuses when --to differs from the frozen max(rowid): the producer is still writing', async () => {
    const f = fakeD1({ rows: ids(8), index: [] });
    const { d } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.SIGNATURE_CHANGED);
    expect(f.writes).toHaveLength(0);
  });

  it('stops when the base signature changes mid-run (AC 2.3)', async () => {
    const state = { rows: ids(7), index: [] as number[] };
    const f = fakeD1(state, { onChunk: (i) => i === 0 && state.rows.push(8) });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.SIGNATURE_CHANGED, lastHi: 3 });
    expect(f.writes).toHaveLength(1);
    expect(lines.join('\n')).toMatch(/SIGNATURE_CHANGED/);
  });

  it('stops at the next chunk boundary when a trigger indexed a row ahead of the backfill (#83)', async () => {
    // An UPDATE leaves (count, max rowid) unchanged but fires exhibits_fts_au on rowid 5.
    const state = { rows: ids(7), index: [] as number[] };
    const f = fakeD1(state, { onChunk: (i) => i === 0 && state.index.push(5) });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.INDEX_STATE, lastHi: 3 });
    expect(f.writes).toHaveLength(1);
    expect(lines.join('\n')).toMatch(/after chunk \(0, 3\].*4.*expected 3/);
  });

  it('stops after three retries of an unhealthy probe (AC 2.4)', async () => {
    const f = fakeD1({ rows: ids(7), index: [] });
    const { d, probes } = deps(f.exec, async () => 500);
    const sleeps: number[] = [];
    d.sleep = async (ms) => {
      sleeps.push(ms);
    };
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.UNHEALTHY, lastHi: 3 });
    expect(probes).toHaveLength(4);
    expect(sleeps).toEqual([1000, 2000, 4000]);
    expect(f.writes).toHaveLength(1);
  });

  it('recovers from a transient probe failure', async () => {
    const f = fakeD1({ rows: ids(7), index: [] });
    let calls = 0;
    const { d } = deps(f.exec, async () => (++calls === 1 ? 503 : 200));
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.DONE);
  });

  it('reports a failed chunk with its range and both possible outcomes (#83)', async () => {
    const f = fakeD1({ rows: ids(7), index: [] }, { failChunk: 1 });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.CHUNK_FAILED, lastHi: 3 });
    const log = lines.join('\n');
    expect(log).toMatch(/CHUNK_FAILED \(3, 6\]/);
    expect(log).toMatch(/--from 3/);
    expect(log).toMatch(/committed/);
    expect(log).not.toMatch(/A failed D1 statement rolls back\./);
  });

  it('stops above the 8 GB storage cap', async () => {
    const f = fakeD1({ rows: ids(7), index: [], size: 8_000_000_001 });
    const { d } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.SIZE_CAP, lastHi: 3 });
  });

  it('fails closed when a remote write does not report size_after (#83)', async () => {
    const f = fakeD1({ rows: ids(7), index: [], size: null });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.SIZE_CAP, lastHi: 3 });
    expect(lines.join('\n')).toMatch(/size_after/);
  });

  it('local runs log an unknown size instead of 0 (#83)', async () => {
    const f = fakeD1({ rows: ids(7), index: [], size: null });
    const { d, lines } = deps(f.exec);
    expect((await runBackfill(d, { mode: 'local', db: 'sec-ex10', from: 0, chunk: 3 })).code).toBe(EXIT.DONE);
    expect(lines.find((l) => l.startsWith('chunk (0, 3]'))).toMatch(/size=n\/a/);
  });

  it('fails when the final document count differs from the row count', async () => {
    const state = { rows: ids(7), index: [] as number[] };
    let docReads = 0;
    // 1 resume read + 3 per-chunk reads; the 5th docsize read is the final one.
    const f = fakeD1(state, {
      onRead: (sql) => {
        if (sql.includes('exhibits_fts_docsize') && ++docReads === 5) state.index.push(1);
      },
    });
    const { d } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.COUNT_MISMATCH);
  });

  it('fails when integrity-check rejects an index whose counts look right (#83)', async () => {
    const f = fakeD1({ rows: ids(7), index: [], corrupt: true });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.INTEGRITY, lastHi: 7 });
    expect(lines.join('\n')).toMatch(/INTEGRITY.*malformed[\s\S]*delete-all/);
    expect(lines.join('\n')).not.toMatch(/DONE/);
  });

  it('does not call a timed-out integrity-check corruption or advise delete-all (#87)', async () => {
    const f = fakeD1({ rows: ids(7), index: [], integrityError: 'D1 DB exceeded its CPU time limit [7429]' });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.EXEC_FAILED, lastHi: 7 });
    const log = lines.join('\n');
    expect(log).toMatch(/integrity-check did not finish/);
    expect(log).not.toMatch(/delete-all/);
  });

  it('never reads INTEGRITY out of wrangler output text (#88)', async () => {
    const f = fakeD1({
      rows: ids(7),
      index: [],
      integrityError: 'wrangler printed no JSON array: warning: cache looks corrupt, refreshing',
    });
    const { d, lines } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.EXEC_FAILED);
    expect(lines.join('\n')).not.toMatch(/delete-all/);
  });

  it('treats the corruption phrase in raw stderr as a failed check, not INTEGRITY (#89)', async () => {
    const f = fakeD1({
      rows: ids(7),
      index: [],
      integrityError: 'network timeout (stderr: last run exited after: database disk image is malformed)',
    });
    const { d, lines } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.EXEC_FAILED);
    expect(lines.join('\n')).not.toMatch(/delete-all/);
  });

  it('maps a failed read before any chunk to EXEC_FAILED, not a crash (#83)', async () => {
    const f = fakeD1(
      { rows: ids(7), index: [] },
      {
        onRead: (sql) => {
          if (sql.includes('exhibits_fts_docsize')) throw new Error('SQLITE_BUSY');
        },
      },
    );
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.EXEC_FAILED, lastHi: 0 });
    expect(lines.join('\n')).toMatch(/EXEC_FAILED.*SQLITE_BUSY/);
    expect(lines.join('\n')).not.toMatch(/A read against D1 failed/);
  });

  it('maps a failed read mid-run to EXEC_FAILED with the last indexed rowid (#83)', async () => {
    let chunks = 0;
    const f = fakeD1(
      { rows: ids(7), index: [] },
      {
        onChunk: () => chunks++,
        onRead: (sql) => {
          if (chunks === 1 && sql.startsWith('SELECT count(*) AS n, max(rowid)')) throw new Error('fetch failed');
        },
      },
    );
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.EXEC_FAILED, lastHi: 3 });
    expect(lines.join('\n')).toMatch(/--from 3/);
  });

  it('local runs never probe and default --to to the current max(rowid)', async () => {
    const f = fakeD1({ rows: ids(7), index: [] });
    const { d, probes } = deps(f.exec);
    expect((await runBackfill(d, { mode: 'local', db: 'sec-ex10', from: 0, chunk: 3 })).code).toBe(EXIT.DONE);
    expect(probes).toEqual([]);
  });
});

describe('execFailure: mapping a failed wrangler run (#89 follow-up)', () => {
  const d1Json =
    '{\n  "error": {\n    "text": "D1_ERROR: database disk image is malformed: SQLITE_CORRUPT",\n    "notes": [{ "text": "[code: 7500]" }]\n  }\n}';

  it('raises D1Error for D1 structured error JSON, even with banners and trailing lines', () => {
    const e = execFailure(
      `🌀 Executing on remote database\n${d1Json}\n✘ [ERROR] done\n`,
      'stderr noise',
      'Command failed',
    );
    expect(e).toBeInstanceOf(D1Error);
    expect(e.message).toBe('D1_ERROR: database disk image is malformed: SQLITE_CORRUPT: [code: 7500]');
  });

  it('falls back to a plain Error from stderr, then the process message', () => {
    const a = execFailure('no json here', 'database disk image is malformed (from a proxy)', 'Command failed');
    expect(a).not.toBeInstanceOf(D1Error);
    expect(a.message).toMatch(/proxy/);
    expect(execFailure(undefined, '  ', 'Command failed').message).toBe('Command failed');
  });

  it('keeps looking past junk objects and unclosed braces (#89 review)', () => {
    expect(execFailure(`{"not":"error"}\n${d1Json}`, '', 'x')).toBeInstanceOf(D1Error);
    expect(execFailure(`{\n${d1Json}`, '', 'x')).toBeInstanceOf(D1Error);
  });

  it('tolerates a non-array notes field', () => {
    const e = execFailure('{"error":{"text":"database disk image is malformed","notes":"oops"}}', '', 'x');
    expect(e).toBeInstanceOf(D1Error);
    expect(e.message).toBe('database disk image is malformed');
  });

  it('stays linear on many unclosed braces', () => {
    const t0 = performance.now();
    expect(execFailure('{\n'.repeat(100_000), 'stderr', 'x')).not.toBeInstanceOf(D1Error);
    expect(performance.now() - t0).toBeLessThan(300);
  });

  it('feeds INTEGRITY end to end only for the D1 path', async () => {
    const run = async (err: Error) => {
      const f = fakeD1({ rows: ids(7), index: [], integrityError: 'unused' });
      const exec: Deps['exec'] = async (sql) => {
        if (sql.includes("'integrity-check'")) throw err;
        return f.exec(sql);
      };
      return (await runBackfill(deps(exec).d, { ...BASE, to: 7 })).code;
    };
    expect(await run(execFailure(d1Json, '', 'x'))).toBe(EXIT.INTEGRITY);
    expect(await run(execFailure('', 'database disk image is malformed', 'x'))).toBe(EXIT.EXEC_FAILED);
  });
});

describe('parseWranglerJson (#83)', () => {
  const row =
    '[\n  {\n    "results": [{ "n": 3 }],\n    "success": true,\n    "meta": { "changes": 2, "size_after": 123, "duration": 1.5 }\n  }\n]\n';

  it('reads results and meta after the remote banner lines', () => {
    const stdout = ` \n🌀 Executing on remote database sec-ex10 (abc):\n🌀 To execute on your local development database, remove the --remote flag [sic].\n${row}`;
    expect(parseWranglerJson(stdout)).toEqual({ rows: [{ n: 3 }], changes: 2, sizeAfter: 123, durationMs: 1.5 });
  });

  it('leaves changes and size undefined when wrangler omits them (local --json)', () => {
    const stdout = '[{"results":[],"success":true,"meta":{"duration":0}}]';
    expect(parseWranglerJson(stdout)).toEqual({ rows: [], changes: undefined, sizeAfter: undefined, durationMs: 0 });
  });

  it('ignores trailing output and banner lines that start with [ (#87)', () => {
    expect(parseWranglerJson(`${row}Done in 412ms\n`).rows).toEqual([{ n: 3 }]);
    expect(parseWranglerJson(`[mf:inf] local db ready\n${row}`).rows).toEqual([{ n: 3 }]);
  });

  it('scans each candidate once, so many bracketed banner lines stay cheap (#88)', () => {
    const banners = '[warn] something odd ]\n'.repeat(2000);
    const t0 = performance.now();
    expect(parseWranglerJson(`${banners}${row}`).rows).toEqual([{ n: 3 }]);
    expect(() => parseWranglerJson(banners)).toThrow(/no JSON/);
    expect(performance.now() - t0).toBeLessThan(500);
  });

  it('gives up quickly on many unclosed line-initial brackets (#89)', () => {
    const t0 = performance.now();
    expect(() => parseWranglerJson(`${'[#### progress\n'.repeat(5000)}${row}`)).toThrow(/no JSON/);
    expect(performance.now() - t0).toBeLessThan(300);
    expect(parseWranglerJson(`[#### progress\n[mf:inf] x\n${row}`).rows).toEqual([{ n: 3 }]);
  });

  it('reports an empty array as empty, not as success=undefined (#88)', () => {
    expect(() => parseWranglerJson('[]')).toThrow(/empty/);
  });

  it('rejects an unsuccessful statement', () => {
    expect(() => parseWranglerJson('[{"results":[],"success":false,"meta":{"duration":0}}]')).toThrow(/success/);
  });

  it('rejects output with no JSON array', () => {
    expect(() => parseWranglerJson('✘ [ERROR] something broke')).toThrow(/no JSON/);
  });
});
