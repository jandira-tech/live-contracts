import type { APIRoute } from 'astro';
import { desc } from 'drizzle-orm';
import { getDb } from '../db/client';
import { exhibits } from '../db/schema';

// SSR sitemap. The static @astrojs/sitemap integration only sees prerendered
// routes (here just /404), so we generate it at request time instead: the fixed
// canonical routes + the most recent agreements from D1, edge-cached so we don't
// hit D1 on every crawl.
export const prerender = false;

const SITE = 'https://live-contracts.arthur.law';
const MAX_AGREEMENTS = 2000; // well under the 50k/50MB sitemap limit

function xmlEscape(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' }[c]!));
}

// SEC acceptance stamps arrive as YYYYMMDD… (or RFC3339); emit a W3C date when
// we can parse one, else omit <lastmod>.
function toLastmod(v: string | null | undefined): string | null {
  if (!v) return null;
  const digits = v.replace(/\D/g, '');
  if (digits.length >= 8) return `${digits.slice(0, 4)}-${digits.slice(4, 6)}-${digits.slice(6, 8)}`;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d.toISOString().slice(0, 10);
}

function urlEntry(loc: string, lastmod: string | null, changefreq: string, priority: string): string {
  return (
    `  <url>\n    <loc>${xmlEscape(loc)}</loc>\n` +
    (lastmod ? `    <lastmod>${lastmod}</lastmod>\n` : '') +
    `    <changefreq>${changefreq}</changefreq>\n    <priority>${priority}</priority>\n  </url>`
  );
}

export const GET: APIRoute = async () => {
  const entries: string[] = [
    urlEntry(`${SITE}/`, null, 'always', '1.0'),
    urlEntry(`${SITE}/agreements/1`, null, 'hourly', '0.8'),
    urlEntry(`${SITE}/search`, null, 'weekly', '0.5'),
  ];

  try {
    const rows = await getDb()
      .select({ id: exhibits.id, filedAt: exhibits.filedAt, foundAt: exhibits.foundAt })
      .from(exhibits)
      .orderBy(desc(exhibits.filedAt))
      .limit(MAX_AGREEMENTS);
    for (const r of rows) {
      if (!r.id) continue;
      entries.push(urlEntry(`${SITE}/agreement/${encodeURIComponent(r.id)}`, toLastmod(r.filedAt ?? r.foundAt), 'monthly', '0.6'));
    }
  } catch {
    // If D1 is briefly unavailable, still serve the static routes (valid sitemap).
  }

  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${entries.join('\n')}\n</urlset>\n`;

  return new Response(body, {
    headers: {
      'Content-Type': 'application/xml; charset=utf-8',
      'Cache-Control': 'public, max-age=3600, stale-while-revalidate=86400',
    },
  });
};
