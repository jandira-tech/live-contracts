import { defineMiddleware } from 'astro:middleware';

// RFC 8288 Link relations for agent discovery, emitted on every HTML document.
// (The /.well-known/api-catalog body itself is a static asset — see
// public/.well-known/api-catalog + public/_headers — since unmatched dot-paths
// don't reach this middleware on the Cloudflare assets adapter.)
const LINKS = [
  '</.well-known/api-catalog>; rel="api-catalog"; type="application/linkset+json"',
  '</.well-known/mcp/server-card.json>; rel="service-desc"; type="application/json"',
  '</sitemap.xml>; rel="sitemap"; type="application/xml"',
];

export const onRequest = defineMiddleware(async (_context, next) => {
  const res = await next();
  const ct = res.headers.get('content-type') ?? '';
  if (ct.includes('text/html')) {
    for (const link of LINKS) res.headers.append('Link', link);
  }
  return res;
});
