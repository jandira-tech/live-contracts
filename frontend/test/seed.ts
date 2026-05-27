import { env } from 'cloudflare:test';
import { drizzle, type DrizzleD1Database } from 'drizzle-orm/d1';
import * as schema from '../src/db/schema';

// Vite glob: load every generated migration as raw SQL (filename is hashed).
const migrations = import.meta.glob('../migrations/*.sql', {
  query: '?raw', import: 'default', eager: true,
}) as Record<string, string>;

export function testDb(): DrizzleD1Database<typeof schema> {
  return drizzle((env as any).DB, { schema });
}

export async function seed() {
  // Storage isolation is per test *file*, not per test, so the table survives
  // across this file's beforeEach hooks. Drop it first to re-apply migrations
  // and re-insert fixtures from a clean slate on every test.
  await (env as any).DB.exec('DROP TABLE IF EXISTS exhibits');
  for (const key of Object.keys(migrations).sort()) {
    for (const stmt of migrations[key].split('--> statement-breakpoint')) {
      const s = stmt.trim();
      if (s) await (env as any).DB.exec(s.replace(/\n/g, ' '));
    }
  }
  const db = testDb();
  await db.insert(schema.exhibits).values([
    {
      id: 1, accession: 'acc-1', cik: '111', formType: '8-K', docType: 'EX-10.1',
      filename: 'a.htm', description: 'Alpha agreement', sequence: '1', filingUrl: 'https://sec/a',
      foundAt: '2026-05-01T00:00:00', filedAt: '20260501120000', markdownStatus: 'done',
      filingMetadata: JSON.stringify({ company_name: 'Alpha Corp', filed_at: '20260501120000', items: [] }),
      imageUrls: '[]', markdown: 'Alpha contract body about leasing.',
    },
    {
      id: 2, accession: 'acc-2', cik: '222', formType: '10-Q', docType: 'EX-10.2',
      filename: 'b.htm', description: 'Beta agreement', sequence: '1', filingUrl: 'https://sec/b',
      foundAt: '2026-05-02T00:00:00', filedAt: '20260502120000', markdownStatus: 'done',
      filingMetadata: JSON.stringify({ company_name: 'Beta LLC', filed_at: '20260502120000', items: [] }),
      imageUrls: '["https://hf/x/p1.jpg"]', markdown: 'Beta agreement leasing terms.',
    },
  ]);
}
