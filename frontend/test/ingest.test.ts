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
it('round-trip: an ingested row is visible via the read layer', async () => {
  await POST(ctx({ rows: [ROW] }, 'secret'));
  const { listEx10 } = await import('../src/lib/api');
  const res = await listEx10(1, 50, {}, testDb());
  expect(res.items.some((i) => i.accession === 'acc-5')).toBe(true);
});
