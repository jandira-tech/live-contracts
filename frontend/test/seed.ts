import { env } from 'cloudflare:test';
import { drizzle, type DrizzleD1Database } from 'drizzle-orm/d1';
import * as schema from '../src/db/schema';

// Vite glob: load every generated migration as raw SQL (filename is hashed).
const migrations = import.meta.glob('../migrations/*.sql', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

export function testDb(): DrizzleD1Database<typeof schema> {
  return drizzle((env as any).DB, { schema });
}

/** Run one statement on the raw D1 binding. */
export const exec = (s: string) => (env as any).DB.exec(s);

/**
 * Drop every derived and base object so migrations re-apply from scratch.
 * The FTS virtual table goes first: it reads through `exhibits` (external content).
 */
export async function resetDb() {
  await exec('DROP TABLE IF EXISTS exhibits_fts');
  await exec('DROP TABLE IF EXISTS exhibits');
}

/**
 * Apply migrations in filename order, limited to prefixes in (after, upTo].
 * `applyMigrations({ upTo: '0002' })` creates rows' tables before FTS exists;
 * `applyMigrations({ after: '0002' })` then adds the index and its triggers.
 */
export async function applyMigrations({ after = '', upTo = '9999' }: { after?: string; upTo?: string } = {}) {
  for (const key of Object.keys(migrations).sort()) {
    const prefix = key.split('/').pop()!.slice(0, 4);
    if (prefix <= after || prefix > upTo) continue;
    for (const stmt of migrations[key].split('--> statement-breakpoint')) {
      // Drop `--` comment lines first: exec() joins lines with spaces, so a leading
      // comment would otherwise swallow the whole statement.
      const s = stmt.replace(/^\s*--.*$/gm, '').trim();
      if (s) await exec(s.replace(/\n/g, ' '));
    }
  }
}

export const FIXTURES = [
  {
    id: '1',
    accession: 'acc-1',
    cik: '111',
    formType: '8-K',
    docType: 'EX-10.1',
    filename: 'a.htm',
    description: 'Alpha agreement',
    sequence: '1',
    filingUrl: 'https://sec/a',
    foundAt: '2026-05-01T00:00:00',
    filedAt: '20260501120000',
    markdownStatus: 'done',
    filingMetadata: JSON.stringify({ company_name: 'Alpha Corp', filed_at: '20260501120000', items: [] }),
    imageUrls: '[]',
    markdown: 'Alpha contract body about leasing.',
  },
  {
    id: '2',
    accession: 'acc-2',
    cik: '222',
    formType: '10-Q',
    docType: 'EX-10.2',
    filename: 'b.htm',
    description: 'Beta agreement',
    sequence: '1',
    filingUrl: 'https://sec/b',
    foundAt: '2026-05-02T00:00:00',
    filedAt: '20260502120000',
    markdownStatus: 'done',
    filingMetadata: JSON.stringify({ company_name: 'Beta LLC', filed_at: '20260502120000', items: [] }),
    imageUrls: '["https://hf/x/p1.jpg"]',
    markdown: 'Beta agreement leasing terms.',
  },
];

export async function seed() {
  // Storage isolation is per test *file*, not per test, so tables survive across
  // this file's beforeEach hooks. Reset first to re-apply migrations and re-insert
  // fixtures from a clean slate on every test.
  await resetDb();
  await applyMigrations();
  await testDb().insert(schema.exhibits).values(FIXTURES);
}
