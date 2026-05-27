import type { APIContext } from 'astro';
import { drizzle } from 'drizzle-orm/d1';
import { sql } from 'drizzle-orm';
import * as schema from '../../db/schema';
import { exhibits, type ExhibitInsert } from '../../db/schema';

export const prerender = false;

interface InRow {
  id: number; accession: string; cik?: string; form_type?: string; doc_type?: string;
  filename: string; description?: string; sequence?: string; filing_url?: string;
  found_at?: string; filed_at?: string; markdown_status?: string;
  filing_metadata?: string; image_urls?: string; markdown?: string;
}

const toInsert = (r: InRow): ExhibitInsert => ({
  id: r.id, accession: r.accession, cik: r.cik ?? null, formType: r.form_type ?? null,
  docType: r.doc_type ?? null, filename: r.filename, description: r.description ?? null,
  sequence: r.sequence ?? null, filingUrl: r.filing_url ?? null, foundAt: r.found_at ?? null,
  filedAt: r.filed_at ?? null, markdownStatus: r.markdown_status ?? null,
  filingMetadata: r.filing_metadata ?? null, imageUrls: r.image_urls ?? null, markdown: r.markdown ?? null,
});

// On (accession, doc_type, filename) conflict, refresh everything except the conflict key.
const CONFLICT_SET = {
  cik: sql`excluded.cik`, formType: sql`excluded.form_type`,
  description: sql`excluded.description`, sequence: sql`excluded.sequence`, filingUrl: sql`excluded.filing_url`,
  foundAt: sql`excluded.found_at`, filedAt: sql`excluded.filed_at`, markdownStatus: sql`excluded.markdown_status`,
  filingMetadata: sql`excluded.filing_metadata`, imageUrls: sql`excluded.image_urls`, markdown: sql`excluded.markdown`,
};

const j = (o: unknown, status: number) =>
  new Response(JSON.stringify(o), { status, headers: { 'Content-Type': 'application/json' } });

export async function POST(context: APIContext): Promise<Response> {
  const env = context.locals.runtime.env as Env;
  if (env.SEC_API_KEY && context.request.headers.get('X-API-Key') !== env.SEC_API_KEY) {
    return j({ error: 'invalid or missing API key' }, 401);
  }
  let body: { rows?: InRow[] };
  try { body = await context.request.json(); } catch { return j({ error: 'invalid JSON' }, 400); }
  const rows = body?.rows;
  if (!Array.isArray(rows)) return j({ error: 'expected { rows: [...] }' }, 400);
  if (rows.length === 0) return j({ accepted: [] }, 200);
  if (rows.length > 200) return j({ error: 'max 200 rows per batch' }, 400);

  const db = drizzle(env.DB, { schema });
  // Chunk to stay well under SQLite's bound-variable limit (15 cols * 50 = 750).
  const accepted: string[] = [];
  for (let i = 0; i < rows.length; i += 50) {
    const chunk = rows.slice(i, i + 50);
    await db.insert(exhibits).values(chunk.map(toInsert))
      .onConflictDoUpdate({ target: [exhibits.accession, exhibits.docType, exhibits.filename], set: CONFLICT_SET });
    for (const r of chunk) accepted.push(r.accession);
  }
  return j({ accepted }, 200);
}
