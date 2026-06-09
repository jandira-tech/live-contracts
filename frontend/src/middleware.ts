import { defineMiddleware } from 'astro:middleware';
import { ex10Since, listEx10, ex10Search, ex10Detail } from './lib/api';
import { prefersMarkdown, markdownResponse, listMarkdown, detailMarkdown } from './lib/markdown';

// RFC 8288 Link relations for agent discovery, emitted on every HTML document.
// (The /.well-known/api-catalog body is a static asset — see public/_headers.)
const LINKS = [
  '</.well-known/api-catalog>; rel="api-catalog"; type="application/linkset+json"',
  '</.well-known/mcp/server-card.json>; rel="service-desc"; type="application/json"',
  '</sitemap.xml>; rel="sitemap"; type="application/xml"',
];

const WINDOW = 60;
const PAGE_SIZE = 12;

// Build the markdown representation for a page URL when an agent asks for it.
// Returns a string (markdown), null (route matched but resource not found), or
// undefined (no markdown handler for this path — fall through to HTML).
async function buildMarkdown(url: URL): Promise<string | null | undefined> {
  const path = url.pathname;
  const sp = url.searchParams;

  if (path === '/' || path === '') {
    const res = await ex10Since(WINDOW);
    return listMarkdown(
      'Live Contracts — newest EX-10 material contracts',
      `Material contract exhibits filed with SEC EDGAR in the last ${WINDOW}s.`,
      res.items,
    );
  }

  const browse = path.match(/^\/agreements\/(\d+)\/?$/);
  if (browse) {
    const page = Math.max(1, Number(browse[1]) || 1);
    const r = await listEx10(page, PAGE_SIZE, {
      form: sp.get('form') || undefined,
      cik: sp.get('cik') || undefined,
      filer: sp.get('filer') || undefined,
      sort: sp.get('sort') === 'oldest' ? 'oldest' : 'newest',
    });
    const filtered = Boolean(sp.get('form') || sp.get('cik') || sp.get('filer'));
    return listMarkdown(
      `Browse EX-10 agreements — page ${r.page} of ${Math.max(1, r.total_pages)}`,
      `${r.total.toLocaleString()} ${filtered ? 'matching' : 'total'} material contract exhibits.`,
      r.items,
    );
  }

  if (path === '/search') {
    const q = (sp.get('q') || '').trim();
    const r = q ? await ex10Search(q, Math.max(1, Number(sp.get('page')) || 1), PAGE_SIZE) : null;
    return listMarkdown(
      q ? `Search results for “${q}”` : 'Search EX-10 agreements',
      q ? `${(r?.total ?? 0).toLocaleString()} result(s) for “${q}”.` : 'Provide ?q= to search across every EX-10 material contract.',
      r?.items ?? [],
    );
  }

  const detail = path.match(/^\/agreement\/([^/]+)\/?$/);
  if (detail) {
    const d = await ex10Detail(decodeURIComponent(detail[1]));
    return d ? detailMarkdown(d) : null;
  }

  return undefined;
}

export const onRequest = defineMiddleware(async (context, next) => {
  // Markdown content negotiation for agents (HTML stays the default for browsers).
  if (prefersMarkdown(context.request.headers.get('accept'))) {
    try {
      const md = await buildMarkdown(context.url);
      if (md === null) return markdownResponse('# Not found\n\nThat agreement isn’t here.\n', 404);
      if (typeof md === 'string') return markdownResponse(md);
      // undefined → no handler for this path; fall through to the HTML response.
    } catch {
      // On any data error, fall through and let the page render its HTML/empty state.
    }
  }

  const res = await next();
  const ct = res.headers.get('content-type') ?? '';
  if (ct.includes('text/html')) {
    for (const link of LINKS) res.headers.append('Link', link);
    // Pages also answer Accept: text/markdown — vary so caches don't cross-serve.
    res.headers.append('Vary', 'Accept');
  }
  return res;
});
