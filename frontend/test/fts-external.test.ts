// External-content FTS5 index (migration 0003) — acceptance items AC 1.1–1.5, 1.9
// from contracts_fix.md §3. Every case runs against the real migration chain.
import { it, expect, beforeEach, describe } from 'vitest';
import { env } from 'cloudflare:test';
import { eq } from 'drizzle-orm';
import { applyMigrations, exec, FIXTURES, resetDb, seed, testDb } from './seed';
import { ex10Search, searchPageSql } from '../src/lib/search';
import { POST } from '../src/pages/api/ingest';
import { exhibits } from '../src/db/schema';

const DB = () => (env as any).DB;
const one = async <T>(q: string) => (await DB().prepare(q).first()) as T;
const integrity = (rank: 0 | 1 = 1) =>
  DB().prepare(`INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', ${rank})`).run();
const segmentMax = async () => (await one<{ m: number | null }>('SELECT max(id) AS m FROM exhibits_fts_data')).m;
const BACKFILL =
  "INSERT INTO exhibits_fts(rowid, description, markdown) SELECT rowid, coalesce(description,''), coalesce(markdown,'') FROM exhibits WHERE rowid > 0 AND rowid <= 50";

describe('rows that predate the index (AC 1.1)', () => {
  beforeEach(async () => {
    await resetDb();
    await applyMigrations({ upTo: '0002' });
    await testDb().insert(exhibits).values(FIXTURES);
    await applyMigrations({ after: '0002' });
  });

  it('are invisible before the backfill and searchable after it', async () => {
    expect((await ex10Search('leasing', 1, 20, testDb())).total).toBe(0);
    await DB().prepare(BACKFILL).run();
    const r = await ex10Search('leasing', 1, 20, testDb());
    expect(r.total).toBe(2);
    expect(r.items.map((i) => i.id)).toEqual(['2', '1']);
    expect(r.items[0].snippet).toContain('\uFDD0leasing\uFDD1');
    await integrity(1);
  });
});

describe('index integrity (AC 1.2, 1.3)', () => {
  beforeEach(seed);

  it('holds after inserts, updates and deletes', async () => {
    const db = testDb();
    await integrity(1);
    await db
      .insert(exhibits)
      .values({ id: '9', accession: 'acc-9', filename: 'z.htm', description: 'Zeta', markdown: 'zebra' });
    await integrity(1);
    await db.update(exhibits).set({ markdown: 'giraffe' }).where(eq(exhibits.id, '9'));
    await integrity(1);
    await db.update(exhibits).set({ description: null }).where(eq(exhibits.id, '9'));
    await integrity(1);
    await db.delete(exhibits).where(eq(exhibits.id, '9'));
    await integrity(1);
    expect((await ex10Search('giraffe', 1, 20, db)).total).toBe(0);
  });

  it('rank=1 rejects a doubly indexed row', async () => {
    await exec(
      "INSERT INTO exhibits_fts(rowid, description, markdown) SELECT rowid, coalesce(description,''), coalesce(markdown,'') FROM exhibits WHERE id = '1'",
    );
    await expect(integrity(1)).rejects.toThrow(/database disk image is malformed/);
  });

  it('rank=1 rejects stale content left by a missing update trigger', async () => {
    await exec('DROP TRIGGER exhibits_fts_au');
    await testDb().update(exhibits).set({ markdown: 'replacement text' }).where(eq(exhibits.id, '1'));
    await expect(integrity(1)).rejects.toThrow(/database disk image is malformed/);
  });
});

describe('ingest re-pushes (AC 1.4)', () => {
  beforeEach(seed);

  const ROW = {
    id: 5,
    accession: 'acc-5',
    cik: '5',
    form_type: '8-K',
    doc_type: 'EX-10.1',
    filename: 'f.htm',
    description: 'Delta lease',
    sequence: '1',
    filing_url: 'u',
    found_at: '2026-05-03T00:00:00',
    filed_at: '20260503120000',
    markdown_status: 'done',
    filing_metadata: '{}',
    image_urls: '[]',
    markdown: 'delta body',
  };
  const push = (row: Record<string, unknown>) =>
    POST({
      request: new Request('https://x/api/ingest', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-API-Key': 'secret' },
        body: JSON.stringify({ rows: [row] }),
      }),
    } as any);

  it('skip the index when description and markdown are unchanged, rewrite it when they change', async () => {
    expect((await push(ROW)).status).toBe(200);
    const afterInsert = await segmentMax();
    expect((await push(ROW)).status).toBe(200);
    expect(await segmentMax()).toBe(afterInsert);

    expect((await push({ ...ROW, markdown: 'delta body revised' })).status).toBe(200);
    const afterChange = await segmentMax();
    expect(afterChange).toBeGreaterThan(afterInsert!);
    expect((await ex10Search('revised', 1, 20, testDb())).total).toBe(1);

    expect((await push({ ...ROW, description: null })).status).toBe(200);
    const afterNull = await segmentMax();
    expect((await push({ ...ROW, description: null })).status).toBe(200);
    expect(await segmentMax()).toBe(afterNull);

    // The NULL-description push also restores ROW's markdown, so "revised" is gone.
    expect((await ex10Search('revised', 1, 20, testDb())).total).toBe(0);
    expect((await ex10Search('delta', 1, 20, testDb())).total).toBe(1);
    await integrity(1);
  });
});

describe('broad terms (AC 1.5)', () => {
  beforeEach(async () => {
    await seed();
    // 300 rows, ~40 KB each, all matching "agreement", filed before the fixtures.
    await exec(
      'WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 300) ' +
        'INSERT INTO exhibits(id, accession, filename, filed_at, description, markdown) ' +
        "SELECT 'bulk-' || i, 'acc-bulk-' || i, 'b' || i || '.htm', printf('2025%010d', i), 'Bulk', " +
        "'agreement ' || replace(hex(zeroblob(20000)), '00', 'x ') FROM n",
    );
  });

  it('return exact totals and one page of snippets, newest first', async () => {
    const db = testDb();
    const p1 = await ex10Search('agreement', 1, 12, db);
    const p2 = await ex10Search('agreement', 2, 12, db);
    expect(p2.total).toBe(302);
    expect(p2.page_size).toBe(12);
    expect(p2.total_pages).toBe(26);
    expect(p2.items).toHaveLength(12);
    expect(p2.items.every((i) => (i.snippet ?? '').includes('\uFDD0agreement\uFDD1'))).toBe(true);
    const dates = [...p1.items, ...p2.items].map((i) => i.filed_at);
    expect(dates).toEqual([...dates].sort((a = '', b = '') => b.localeCompare(a)));
    expect(new Set([...p1.items, ...p2.items].map((i) => i.id)).size).toBe(24);
    const last = await ex10Search('agreement', 26, 12, db);
    expect(last.items).toHaveLength(302 - 25 * 12);
  });

  it('compute snippet() only for the page, via a rowid subquery with the LIMIT inside', () => {
    const q = searchPageSql(testDb(), '"agreement"', 12, 12).toLowerCase().replace(/\s+/g, ' ');
    // -1: FTS5 takes the snippet from whichever column matched (#86).
    expect(q).toMatch(/snippet\(exhibits_fts, -1,/);
    expect(q.split('snippet(')).toHaveLength(2);
    expect(q.indexOf('snippet(')).toBeLessThan(q.indexOf(' from exhibits_fts'));
    expect(q).toMatch(/\.rowid in \(select .* limit \? offset \?\)/);
    expect(q).not.toContain('exhibits"."id" = exhibits_fts.id');
  });
});

describe('seed loop (AC 1.9)', () => {
  it('re-seeding twice leaves a consistent index', async () => {
    await seed();
    await seed();
    await integrity(1);
    expect((await ex10Search('leasing', 1, 20, testDb())).total).toBe(2);
  });
});
