import type { APIContext } from 'astro';
import { eq } from 'drizzle-orm';
import { getDb } from '../../../db/client';
import { exhibits } from '../../../db/schema';

export const prerender = false;

// Public raw-markdown endpoint. Returns an exhibit's Markdown body as plain
// text/markdown so external tools (e.g. Cicero's "Export to Cicero" import)
// can fetch it cross-origin. Public SEC data — open CORS, edge-cached.
export async function GET(context: APIContext): Promise<Response> {
  const id = Number(context.params.id);
  if (!Number.isInteger(id) || id <= 0) return notFound();
  const row = (
    await getDb()
      .select({ markdown: exhibits.markdown })
      .from(exhibits)
      .where(eq(exhibits.id, id))
      .limit(1)
  )[0];
  if (!row?.markdown) return notFound();
  return new Response(row.markdown, {
    status: 200,
    headers: {
      'Content-Type': 'text/markdown; charset=utf-8',
      'Access-Control-Allow-Origin': '*',
      'Cache-Control': 'public, max-age=300, stale-while-revalidate=600',
    },
  });
}

function notFound(): Response {
  return new Response('not found', { status: 404 });
}
