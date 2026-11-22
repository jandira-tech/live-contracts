import { it, expect, beforeEach } from 'vitest';
import { env } from 'cloudflare:test';
import { seed, testDb } from './seed';
import { POST } from '../src/pages/api/ingest';
import { exhibits } from '../src/db/schema';
import { eq } from 'drizzle-orm';
import { compileRedactions, redactPersonalData, REPLACEMENT } from '../src/lib/redactions';

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
  expect(redactPersonalData(CELL, compileRedactions(undefined))).toBe(CELL);
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

it('a re-push of the original contract cannot put the address back', async () => {
  const row = {
    id: 1,
    accession: 'acc-gdpr',
    doc_type: 'EX-10.11',
    filename: 'ex10_11.htm',
    description: 'EX-10.11',
    markdown_status: 'done',
    markdown: `Dienstvertrag\n${CELL}\nPräambel`,
  };
  expect((await POST(ctx({ rows: [row] }))).status).toBe(200);
  const stored = (await testDb().select().from(exhibits).where(eq(exhibits.filename, row.filename)))[0];
  expect(stored.markdown).toContain('Dr Max Mustermann');
  expect(stored.markdown).not.toMatch(/Examplestra|12345|Exämpletown/);
});
