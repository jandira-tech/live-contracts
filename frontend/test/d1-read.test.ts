import { describe, it, expect, beforeEach } from 'vitest';
import { seed, testDb } from './seed';
import { listEx10, ex10Detail, ex10Facets, ex10Search, ex10Since, ex10Stats, ex10Freshness } from '../src/lib/api';
import { exhibits } from '../src/db/schema';

let db: ReturnType<typeof testDb>;
beforeEach(async () => { await seed(); db = testDb(); });

it('listEx10: newest-by-filed_at first, with paging + filing fields', async () => {
  const res = await listEx10(1, 20, {}, db);
  expect(res.total).toBe(2);
  expect(res.items.map((i) => i.id)).toEqual(['2', '1']);
  expect(res.items[0].excerpt).toContain('Beta');
  expect(res.items[0].company_name).toBe('Beta LLC');
});
it('listEx10: filters by form_type', async () => {
  const res = await listEx10(1, 20, { form: '8-K' }, db);
  expect(res.total).toBe(1); expect(res.items[0].id).toBe('1');
});
it('ex10Detail: full markdown + parsed filing + images', async () => {
  const d = await ex10Detail('2', db);
  expect(d?.markdown).toContain('Beta agreement');
  expect(d?.filing?.company_name).toBe('Beta LLC');
  expect(d?.image_urls).toEqual(['https://hf/x/p1.jpg']);
});
it('ex10Detail: null for missing id', async () => { expect(await ex10Detail('999', db)).toBeNull(); });
it('ex10Search: matches body, newest first; empty query → none', async () => {
  expect((await ex10Search('leasing', 1, 20, db)).items[0].id).toBe('2');
  expect((await ex10Search('   ', 1, 20, db)).total).toBe(0);
});
it('ex10Facets / ex10Stats', async () => {
  const f = await ex10Facets(db);
  expect(f.forms).toEqual(expect.arrayContaining([{ form_type: '8-K', count: 1 }, { form_type: '10-Q', count: 1 }]));
  const s = await ex10Stats(db);
  expect(s.total).toBe(2); expect(s.with_markdown).toBe(2);
});
it('ex10Since includes a fresh row and excludes the old seed rows', async () => {
  const nowSql = new Date().toISOString().replace('T', ' ').slice(0, 19); // 'YYYY-MM-DD HH:MM:SS'
  await db.insert(exhibits).values({
    id: '99', accession: 'fresh', filename: 'n.htm', docType: 'EX-10.9', formType: '8-K',
    foundAt: nowSql, filedAt: '20260527120000', markdownStatus: 'done',
    filingMetadata: '{}', imageUrls: '[]', markdown: 'fresh body',
  });
  const res = await ex10Since(3600, db);
  expect(res.items.map((i) => i.accession)).toEqual(['fresh']); // seed rows are months old
});

describe('ex10Freshness (GET /stats.json)', () => {
  it('reports the row count and the newest filed_at / detected_at', async () => {
    // Seed rows carry no detected_at (pre-enrichment producer) — it must read as null,
    // never as an empty string that a staleness check would mistake for a date.
    expect(await ex10Freshness(db)).toEqual({
      exhibits: 2, latest_filed_at: '20260502120000', latest_detected_at: null,
    });
    await db.insert(exhibits).values([
      { id: '3', accession: 'acc-3', filename: 'c.htm', docType: 'EX-10.1', filedAt: '20260930170000',
        detectedAt: '2026-09-30T21:00:05+00:00', markdown: 'x' },
      // An older filing detected later (a backfill): the two maxima are independent.
      { id: '4', accession: 'acc-4', filename: 'd.htm', docType: 'EX-10.1', filedAt: '20260810090000',
        detectedAt: '2026-10-02T09:00:00+00:00', markdown: 'y' },
      // A blank filed_at must not win MAX() or be reported.
      { id: '5', accession: 'acc-5', filename: 'e.htm', docType: 'EX-10.1', filedAt: '', markdown: 'z' },
    ]);
    expect(await ex10Freshness(db)).toEqual({
      exhibits: 5, latest_filed_at: '20260930170000', latest_detected_at: '2026-10-02T09:00:00+00:00',
    });
  });

  it('an empty table reports zero and nulls rather than throwing', async () => {
    await db.delete(exhibits);
    expect(await ex10Freshness(db)).toEqual({ exhibits: 0, latest_filed_at: null, latest_detected_at: null });
  });
});
