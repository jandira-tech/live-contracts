// Backfill core (AC 2.1–2.5 in contracts_fix.md §4). Pure functions with injected
// deps: no child processes run inside the Workers pool.
import { describe, it, expect } from 'vitest';
import { EXIT, parseArgs, planRanges, runBackfill, type Deps, type ExecResult } from '../scripts/fts-backfill-core';

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
  ])('rejects %j', (argv) => {
    expect(() => parseArgs(argv)).toThrow(/usage/i);
  });
});

/** A scripted D1: answers the backfill's read queries from `state`, records every write. */
function fakeD1(
  state: { n: number; m: number; docs: number; below?: number; size?: number },
  hooks: { onChunk?: (i: number) => void; failChunk?: number } = {},
) {
  const writes: string[] = [];
  const exec = async (sql: string): Promise<ExecResult> => {
    const r = (rows: Record<string, unknown>[], changes = 0): ExecResult => ({
      rows,
      changes,
      sizeAfter: state.size ?? 1000,
      durationMs: 5,
    });
    if (/^SELECT count\(\*\) AS n, max\(rowid\) AS m FROM exhibits$/.test(sql)) return r([{ n: state.n, m: state.m }]);
    if (/FROM exhibits_fts_docsize/.test(sql)) return r([{ n: state.docs }]);
    if (/^SELECT count\(\*\) AS n FROM exhibits WHERE rowid <= \d+$/.test(sql)) return r([{ n: state.below ?? 0 }]);
    if (sql.startsWith('INSERT INTO exhibits_fts(rowid, description, markdown)')) {
      const i = writes.length;
      writes.push(sql);
      if (hooks.failChunk === i) throw new Error('D1_ERROR: boom');
      const [lo, hi] = sql
        .match(/rowid > (\d+) AND rowid <= (\d+)/)!
        .slice(1)
        .map(Number);
      state.docs += hi - lo;
      hooks.onChunk?.(i);
      return r([], hi - lo);
    }
    throw new Error(`unexpected SQL: ${sql}`);
  };
  return { exec, writes };
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
  it('indexes every range and ends with DONE (AC 2.5 from 0)', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 0 });
    const { d, lines, probes } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.DONE, lastHi: 7 });
    expect(f.writes).toHaveLength(3);
    expect(f.writes[0]).toMatch(/WHERE rowid > 0 AND rowid <= 3$/);
    expect(f.writes[0]).not.toMatch(/NOT EXISTS/);
    expect(probes).toEqual(['https://o/stats.json', 'https://o/stats.json', 'https://o/stats.json']);
    expect(lines.at(-1)).toBe('DONE to=7');
  });

  it('resumes from --from when the index holds exactly the rows at or below it (AC 2.5)', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 6, below: 6 });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, from: 6, to: 7 })).toEqual({ code: EXIT.DONE, lastHi: 7 });
    expect(f.writes).toHaveLength(1);
    expect(f.writes[0]).toMatch(/rowid > 6 AND rowid <= 7$/);
    expect(lines.at(-1)).toBe('DONE to=7');
  });

  it('refuses a fresh run over a non-empty index', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 2 });
    const { d, lines } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.INDEX_STATE);
    expect(f.writes).toHaveLength(0);
    expect(lines.join('\n')).toMatch(/delete-all/);
  });

  it('refuses to resume when an unlogged chunk already committed', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 6, below: 3 });
    const { d } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, from: 3, to: 7 })).code).toBe(EXIT.INDEX_STATE);
    expect(f.writes).toHaveLength(0);
  });

  it('refuses when --to differs from the frozen max(rowid): the producer is still writing', async () => {
    const f = fakeD1({ n: 8, m: 8, docs: 0 });
    const { d } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.SIGNATURE_CHANGED);
    expect(f.writes).toHaveLength(0);
  });

  it('stops when the base signature changes mid-run (AC 2.3)', async () => {
    const state = { n: 7, m: 7, docs: 0 };
    const f = fakeD1(state, {
      onChunk: (i) => {
        if (i === 0) {
          state.n = 8;
          state.m = 8;
        }
      },
    });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.SIGNATURE_CHANGED, lastHi: 3 });
    expect(f.writes).toHaveLength(1);
    expect(lines.join('\n')).toMatch(/SIGNATURE_CHANGED/);
  });

  it('stops after three retries of an unhealthy probe (AC 2.4)', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 0 });
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
    const f = fakeD1({ n: 7, m: 7, docs: 0 });
    let calls = 0;
    const { d } = deps(f.exec, async () => (++calls === 1 ? 503 : 200));
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.DONE);
  });

  it('reports a failed chunk with its range and the recovery path', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 0 }, { failChunk: 1 });
    const { d, lines } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.CHUNK_FAILED, lastHi: 3 });
    expect(lines.join('\n')).toMatch(/--from 3/);
  });

  it('stops above the 8 GB storage cap', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 0, size: 8_000_000_001 });
    const { d } = deps(f.exec);
    expect(await runBackfill(d, { ...BASE, to: 7 })).toEqual({ code: EXIT.SIZE_CAP, lastHi: 3 });
  });

  it('fails when the final document count differs from the row count', async () => {
    const f = fakeD1({ n: 6, m: 7, docs: 0 }); // a rowid gap: 6 rows, max rowid 7; fake indexes 7
    const { d } = deps(f.exec);
    expect((await runBackfill(d, { ...BASE, to: 7 })).code).toBe(EXIT.COUNT_MISMATCH);
  });

  it('local runs never probe and default --to to the current max(rowid)', async () => {
    const f = fakeD1({ n: 7, m: 7, docs: 0 });
    const { d, probes } = deps(f.exec);
    expect((await runBackfill(d, { mode: 'local', db: 'sec-ex10', from: 0, chunk: 3 })).code).toBe(EXIT.DONE);
    expect(probes).toEqual([]);
  });
});
