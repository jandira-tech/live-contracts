import type { APIContext } from 'astro';
import { env } from 'cloudflare:workers'; // Astro v6 removed Astro.locals.runtime.env
import { drizzle } from 'drizzle-orm/d1';
import { sql } from 'drizzle-orm';
import * as schema from '../../db/schema';
import { exhibits, type ExhibitInsert } from '../../db/schema';

export const prerender = false;

interface InRow {
  // `id` is the producer's LOCAL SQLite id (volatile — resets on reseed). It is
  // echoed back in `accepted` so the worker can mark its own rows mirrored, but it
  // is NOT used as the D1 primary key (that caused cross-reseed PK collisions).
  id: number; accession: string; cik?: string; form_type?: string; doc_type?: string;
  filename: string; description?: string; sequence?: string; filing_url?: string;
  found_at?: string; filed_at?: string; markdown_status?: string;
  filing_metadata?: string; image_urls?: string; markdown?: string;
}

// UUIDv7: 48-bit big-endian unix-ms timestamp + 74 random bits, version/variant set.
// Globally unique (no collisions across producer reseeds) and lexicographically
// time-ordered (sortable like the old autoincrement id). Assigned to NEW rows only;
// on (accession, doc_type, filename) conflict the existing id is kept (not in the
// update set), so a row's id is stable across re-pushes.
function uuidv7(): string {
  const ms = Date.now();
  const b = new Uint8Array(16);
  crypto.getRandomValues(b);
  b[0] = Math.floor(ms / 2 ** 40) & 0xff;
  b[1] = Math.floor(ms / 2 ** 32) & 0xff;
  b[2] = Math.floor(ms / 2 ** 24) & 0xff;
  b[3] = Math.floor(ms / 2 ** 16) & 0xff;
  b[4] = Math.floor(ms / 2 ** 8) & 0xff;
  b[5] = ms & 0xff;
  b[6] = (b[6] & 0x0f) | 0x70; // version 7
  b[8] = (b[8] & 0x3f) | 0x80; // variant 10
  const h = Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

const toInsert = (r: InRow): ExhibitInsert => ({
  id: uuidv7(), accession: r.accession, cik: r.cik ?? null, formType: r.form_type ?? null,
  docType: r.doc_type ?? null, filename: r.filename, description: r.description ?? null,
  sequence: r.sequence ?? null, filingUrl: r.filing_url ?? null, foundAt: r.found_at ?? null,
  filedAt: r.filed_at ?? null, markdownStatus: r.markdown_status ?? null,
  filingMetadata: r.filing_metadata ?? null, imageUrls: r.image_urls ?? null, markdown: r.markdown ?? null,
});

// On (accession, doc_type, filename) conflict, refresh everything except the conflict key.
// Enrich-only for the progressively-filled columns: COALESCE(excluded.x, x) so an
// incoming NULL never erases a value already in D1 — a partial/early decoupled push
// (markdown+metadata before image capture) or a stale reseed re-push must not wipe
// captured image_urls/markdown/metadata. A real (non-NULL) incoming value still wins.
const CONFLICT_SET = {
  cik: sql`excluded.cik`, formType: sql`excluded.form_type`,
  description: sql`excluded.description`, sequence: sql`excluded.sequence`, filingUrl: sql`excluded.filing_url`,
  foundAt: sql`excluded.found_at`,
  filedAt: sql`coalesce(excluded.filed_at, filed_at)`,
  markdownStatus: sql`coalesce(excluded.markdown_status, markdown_status)`,
  filingMetadata: sql`coalesce(excluded.filing_metadata, filing_metadata)`,
  imageUrls: sql`coalesce(excluded.image_urls, image_urls)`,
  markdown: sql`coalesce(excluded.markdown, markdown)`,
};

const j = (o: unknown, status: number) =>
  new Response(JSON.stringify(o), { status, headers: { 'Content-Type': 'application/json' } });

export async function POST(context: APIContext): Promise<Response> {
  const e = env as unknown as Env;
  // Fail closed: a missing key is a misconfiguration, not an open door.
  const key = e.SEC_API_KEY;
  if (!key) return j({ error: 'server misconfigured: SEC_API_KEY unset' }, 500);
  if (context.request.headers.get('X-API-Key') !== key) {
    return j({ error: 'invalid or missing API key' }, 401);
  }
  let body: { rows?: InRow[] };
  try { body = await context.request.json(); } catch { return j({ error: 'invalid JSON' }, 400); }
  const rows = body?.rows;
  if (!Array.isArray(rows)) return j({ error: 'expected { rows: [...] }' }, 400);
  if (rows.length === 0) return j({ accepted: [] }, 200);
  if (rows.length > 200) return j({ error: 'max 200 rows per batch' }, 400);

  const db = drizzle(e.DB, { schema });
  // D1 caps bound parameters at 100/query → ≤6 rows/insert (15 cols * 6 = 90).
  // Run the chunks in one atomic db.batch (fewer round-trips, no partial write).
  // Return the unique ids accepted (NOT accessions — accession isn't unique, so
  // the writer must mark mirrored by id to avoid dropping same-accession rows).
  const accepted: number[] = [];
  const stmts = [];
  for (let i = 0; i < rows.length; i += 6) {
    const chunk = rows.slice(i, i + 6);
    stmts.push(
      db.insert(exhibits).values(chunk.map(toInsert))
        .onConflictDoUpdate({ target: [exhibits.accession, exhibits.docType, exhibits.filename], set: CONFLICT_SET }),
    );
    for (const r of chunk) accepted.push(r.id);
  }
  await db.batch(stmts as unknown as Parameters<typeof db.batch>[0]);
  return j({ accepted }, 200);
}
