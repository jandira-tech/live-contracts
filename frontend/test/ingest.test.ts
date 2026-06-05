import { describe, it, expect, beforeEach } from 'vitest';
import { env } from 'cloudflare:test';
import { seed, testDb } from './seed';   // seed() also applies migrations
import { POST } from '../src/pages/api/ingest';
import { exhibits } from '../src/db/schema';
import { eq } from 'drizzle-orm';

beforeEach(async () => { await seed(); });

function ctx(body: unknown, key?: string) {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (key) headers['X-API-Key'] = key;
  return {
    request: new Request('https://x/api/ingest', { method: 'POST', headers, body: JSON.stringify(body) }),
    // Spread the real cloudflare:test env so env.DB is the live D1 binding.
    locals: { runtime: { env: { ...env, SEC_API_KEY: 'secret' } } },
  } as any;
}
const ROW = {
  id: 5, accession: 'acc-5', cik: '5', form_type: '8-K', doc_type: 'EX-10.1', filename: 'f.htm',
  description: 'D', sequence: '1', filing_url: 'u', found_at: '2026-05-03T00:00:00',
  filed_at: '20260503120000', markdown_status: 'done', filing_metadata: '{}', image_urls: '[]', markdown: 'body',
};

it('401 without the key', async () => { expect((await POST(ctx({ rows: [ROW] }))).status).toBe(401); });
it('400 on malformed body', async () => { expect((await POST(ctx({ nope: 1 }, 'secret'))).status).toBe(400); });
it('upserts and is idempotent', async () => {
  const r1 = await POST(ctx({ rows: [ROW] }, 'secret'));
  expect(r1.status).toBe(200);
  expect(await r1.json()).toMatchObject({ accepted: [5] }); // unique ids, not accessions
  await POST(ctx({ rows: [{ ...ROW, markdown: 'updated' }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(row.markdown).toBe('updated');           // updated in place
  const all = await db.select().from(exhibits);
  expect(all.filter((r) => r.accession === 'acc-5').length).toBe(1);  // no dup
});
it('enrich-only: a NULL-image re-push must not clobber image_urls/markdown already in D1', async () => {
  // First push: a fully captured row (images + markdown present).
  await POST(ctx({ rows: [{ ...ROW, image_urls: '["https://hf/x.jpg"]', markdown: 'full body' }] }, 'secret'));
  // Re-push the SAME key with NULLs (a reseed / early decoupled push before image capture).
  await POST(ctx({ rows: [{ ...ROW, image_urls: null, markdown: null }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(row.imageUrls).toBe('["https://hf/x.jpg"]'); // preserved, NOT clobbered to NULL
  expect(row.markdown).toBe('full body');             // preserved
});
it('a real (non-NULL) incoming value still wins over the existing one', async () => {
  await POST(ctx({ rows: [{ ...ROW, image_urls: '[]', markdown: 'old' }] }, 'secret'));
  await POST(ctx({ rows: [{ ...ROW, image_urls: '["https://hf/new.jpg"]', markdown: 'new' }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(row.imageUrls).toBe('["https://hf/new.jpg"]'); // non-NULL incoming wins
  expect(row.markdown).toBe('new');
});
const UUIDV7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

it('assigns a UUIDv7 string id to a new row (not the worker-supplied id)', async () => {
  await POST(ctx({ rows: [{ ...ROW, id: 5 }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(typeof row.id).toBe('string');
  expect(row.id).toMatch(UUIDV7);
  expect(row.id).not.toBe('5');                 // the volatile worker id is NOT the PK
});
it('no PK collision when the worker reuses an id across distinct rows (the reseed bug)', async () => {
  // Same worker id (1) on two DIFFERENT exhibits — used to throw a PRIMARY KEY conflict.
  const res = await POST(ctx({ rows: [
    { ...ROW, id: 1, accession: 'a1', filename: 'f1.htm' },
    { ...ROW, id: 1, accession: 'a2', filename: 'f2.htm' },
  ] }, 'secret'));
  expect(res.status).toBe(200);
  const db = testDb();
  const r1 = (await db.select().from(exhibits).where(eq(exhibits.accession, 'a1')))[0];
  const r2 = (await db.select().from(exhibits).where(eq(exhibits.accession, 'a2')))[0];
  expect(r1.id).toMatch(UUIDV7);
  expect(r2.id).toMatch(UUIDV7);
  expect(r1.id).not.toBe(r2.id);                // distinct uuids, both inserted
});
it('re-push of the same row keeps its assigned id (stable across pushes)', async () => {
  await POST(ctx({ rows: [{ ...ROW, id: 7 }] }, 'secret'));
  const db = testDb();
  const before = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0].id;
  await POST(ctx({ rows: [{ ...ROW, id: 7, markdown: 'v2' }] }, 'secret'));
  const after = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0].id;
  expect(after).toBe(before);                   // id assigned once, kept on conflict-update
});
it('persists source, size_bytes, detected_at', async () => {
  await POST(ctx({ rows: [{ ...ROW, source: 'efts', size_bytes: 12345, detected_at: '2026-05-03T00:00:00.123Z' }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(row.source).toBe('efts');
  expect(row.sizeBytes).toBe(12345);
  expect(row.detectedAt).toBe('2026-05-03T00:00:00.123Z');
});
it('enrich-only: a re-push with blank/NULL source/size_bytes/detected_at does NOT wipe stored values', async () => {
  await POST(ctx({ rows: [{ ...ROW, source: 'rss', size_bytes: 999, detected_at: '2026-05-03T01:02:03.000Z' }] }, 'secret'));
  // Re-push the SAME key with blank/NULL fields: blank strings normalize to null → coalesce keeps stored.
  await POST(ctx({ rows: [{ ...ROW, source: ' ', size_bytes: null, detected_at: '' }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(row.source).toBe('rss');                    // preserved
  expect(row.sizeBytes).toBe(999);                   // preserved
  expect(row.detectedAt).toBe('2026-05-03T01:02:03.000Z'); // preserved
});
it('a real (non-NULL) source overwrites, while a NULL size_bytes leaves size_bytes intact', async () => {
  await POST(ctx({ rows: [{ ...ROW, source: 'rss', size_bytes: 999 }] }, 'secret'));
  await POST(ctx({ rows: [{ ...ROW, source: 'efts', size_bytes: null }] }, 'secret'));
  const db = testDb();
  const row = (await db.select().from(exhibits).where(eq(exhibits.accession, 'acc-5')))[0];
  expect(row.source).toBe('efts');   // non-NULL incoming wins
  expect(row.sizeBytes).toBe(999);   // NULL incoming → coalesce keeps stored
});
it('accepts a batch of 6 distinct rows (exceeds D1 100-param cap at chunk=6, 18 cols)', async () => {
  const rows = Array.from({ length: 6 }, (_, i) => ({
    ...ROW, id: 100 + i, accession: `batch-${i}`, filename: `b${i}.htm`,
  }));
  const res = await POST(ctx({ rows }, 'secret'));
  expect(res.status).toBe(200);
  expect(((await res.json()) as { accepted: unknown[] }).accepted).toHaveLength(6);
  const db = testDb();
  for (let i = 0; i < 6; i++) {
    const row = (await db.select().from(exhibits).where(eq(exhibits.accession, `batch-${i}`)))[0];
    expect(row).toBeDefined();
  }
});
it('round-trip: an ingested row is visible via the read layer', async () => {
  await POST(ctx({ rows: [ROW] }, 'secret'));
  const { listEx10 } = await import('../src/lib/api');
  const res = await listEx10(1, 50, {}, testDb());
  expect(res.items.some((i) => i.accession === 'acc-5')).toBe(true);
});
