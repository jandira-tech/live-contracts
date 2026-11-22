import { it, expect, beforeEach } from 'vitest';
import { env } from 'cloudflare:test';
import { seed, testDb } from './seed';
import { POST } from '../src/pages/api/ingest';
import { exhibits } from '../src/db/schema';
import { eq } from 'drizzle-orm';
import { compileRedactions, redactJsonText, redactPersonalData, REPLACEMENT } from '../src/lib/redactions';

// Fictional people and addresses only: the real patterns live in the PII_REDACTIONS
// Worker secret and must never be committed to this public repository.
const RULES = compileRedactions(JSON.stringify(['Examplestra(?:ß|ss)e\\s*12(?!\\d)', '12345\\s*Ex(?:ä|ae|a)mpletown']));
const CELL = '| Herrn/Mr Dr Max Mustermann  Examplestraße 12  12345 Exämpletown,  Deutschland/Germany | | |';

it('replaces each address line and keeps the name and country', () => {
  expect(redactPersonalData(CELL, RULES)).toBe(
    `| Herrn/Mr Dr Max Mustermann  ${REPLACEMENT}  ${REPLACEMENT},  Deutschland/Germany | | |`,
  );
});

it('catches a re-conversion with different spacing or spelling', () => {
  for (const text of ['Examplestraße  12', 'Examplestrasse 12,', '12345  Exaempletown', '12345 Exampletown']) {
    expect(redactPersonalData(text, RULES)).toBe(text.replace(/^[^,]+/, REPLACEMENT));
  }
  expect(redactPersonalData('Examplestraße 123', RULES)).toBe('Examplestraße 123'); // a different house
});

it('leaves unrelated text, nulls and an empty list alone', () => {
  expect(redactPersonalData('Nymphenburger Straße 5 80335 München', RULES)).toBe(
    'Nymphenburger Straße 5 80335 München',
  );
  expect(redactPersonalData(null, RULES)).toBeNull();
  expect(redactPersonalData(CELL, compileRedactions('[]'))).toBe(CELL);
});

it('treats a missing or blank secret as misconfigured, not as no rules', () => {
  for (const bad of [undefined, null, '', '   ']) {
    expect(() => compileRedactions(bad)).toThrow('PII_REDACTIONS must be a JSON array');
  }
});

it('redacts strings inside JSON metadata, escaped or not, and keeps untouched JSON byte-identical', () => {
  const escaped = '{"parties":[{"name":"Dr Max Mustermann","address":"Examplestra\\u00dfe 12"}]}';
  const out = JSON.parse(redactJsonText(escaped, RULES)!);
  expect(out.parties[0]).toEqual({ name: 'Dr Max Mustermann', address: REPLACEMENT });
  const clean = '{ "a": "Nymphenburger Straße 5" }';
  expect(redactJsonText(clean, RULES)).toBe(clean);
  expect(redactJsonText('Examplestraße 12, not json', RULES)).toBe(`${REPLACEMENT}, not json`);
});

it('rejects a malformed list instead of silently redacting nothing', () => {
  for (const bad of ['not json', '{"a":1}', '[1]', '[""]', '["("]']) {
    expect(() => compileRedactions(bad)).toThrow('PII_REDACTIONS must be a JSON array');
  }
});

beforeEach(async () => {
  await seed();
});

function ctx(body: unknown) {
  return {
    request: new Request('https://x/api/ingest', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': 'secret' },
      body: JSON.stringify(body),
    }),
    locals: { runtime: { env: { ...env, SEC_API_KEY: 'secret' } } },
  } as any;
}

it('a new row arrives redacted in markdown and metadata', async () => {
  const row = {
    id: 1,
    accession: 'acc-gdpr',
    doc_type: 'EX-10.11',
    filename: 'ex10_11.htm',
    description: 'EX-10.11',
    markdown_status: 'done',
    markdown: `Dienstvertrag\n${CELL}\nPräambel`,
    filing_metadata: JSON.stringify({ signatory: 'Dr Max Mustermann, Examplestraße 12' }),
  };
  expect((await POST(ctx({ rows: [row] }))).status).toBe(200);
  const stored = (await testDb().select().from(exhibits).where(eq(exhibits.filename, row.filename)))[0];
  expect(stored.markdown).toContain('Dr Max Mustermann');
  expect(stored.markdown).not.toMatch(/Examplestra|12345|Exämpletown/);
  expect(stored.filingMetadata).toContain('Dr Max Mustermann');
  expect(stored.filingMetadata).not.toMatch(/Examplestra/);
});

const ftsHits = async (term: string) =>
  (
    await (env as any).DB.prepare('SELECT count(*) AS n FROM exhibits_fts WHERE exhibits_fts MATCH ?')
      .bind(term)
      .first()
  ).n;

it('a re-push over a stored unredacted row cannot put the address back, in D1 or in FTS', async () => {
  const key = { accession: 'acc-gdpr-old', docType: 'EX-10.11', filename: 'old.htm' };
  // A row written before the rule existed (insert directly, bypassing ingest).
  await testDb()
    .insert(exhibits)
    .values({ id: 'gdpr-old', ...key, markdown: `Vertrag ${CELL}` });
  expect(await ftsHits('Examplestraße')).toBe(1);

  const row = {
    id: 7,
    accession: key.accession,
    doc_type: key.docType,
    filename: key.filename,
    markdown: `Vertrag ${CELL}`,
  };
  expect((await POST(ctx({ rows: [row] }))).status).toBe(200);
  const read = async () => (await testDb().select().from(exhibits).where(eq(exhibits.filename, key.filename)))[0];
  expect((await read()).markdown).not.toMatch(/Examplestra|12345/);
  expect(await ftsHits('Examplestraße')).toBe(0);
  expect(await ftsHits('Mustermann')).toBe(1);

  // A later push without markdown keeps the redacted text (coalesce), it does not resurrect anything.
  expect((await POST(ctx({ rows: [{ ...row, markdown: undefined }] }))).status).toBe(200);
  expect((await read()).markdown).toContain(REPLACEMENT);
});
