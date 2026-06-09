import { defineMiddleware } from 'astro:middleware';

const SITE = 'https://live-contracts.arthur.law';

// RFC 9727 API catalog (application/linkset+json). Served from middleware so we
// control the content-type (a static file under /.well-known has no extension to
// infer it from). Advertises the MCP API + its server-card and the sitemap.
const API_CATALOG = {
  linkset: [
    {
      anchor: `${SITE}/mcp`,
      'service-doc': [{ href: `${SITE}/.well-known/mcp/server-card.json`, type: 'application/json' }],
      describedby: [{ href: `${SITE}/.well-known/mcp/server-card.json`, type: 'application/json' }],
    },
    {
      anchor: `${SITE}/`,
      'service-doc': [{ href: `${SITE}/`, type: 'text/html' }],
      related: [{ href: `${SITE}/sitemap.xml`, type: 'application/xml' }],
    },
  ],
};

// RFC 8288 Link relations for agent discovery, emitted on every HTML document.
const LINKS = [
  '</.well-known/api-catalog>; rel="api-catalog"; type="application/linkset+json"',
  '</.well-known/mcp/server-card.json>; rel="service-desc"; type="application/json"',
  '</sitemap.xml>; rel="sitemap"; type="application/xml"',
];

export const onRequest = defineMiddleware(async (context, next) => {
  if (context.url.pathname === '/.well-known/api-catalog') {
    return new Response(JSON.stringify(API_CATALOG, null, 2), {
      headers: { 'content-type': 'application/linkset+json', 'cache-control': 'public, max-age=86400' },
    });
  }
  const res = await next();
  const ct = res.headers.get('content-type') ?? '';
  if (ct.includes('text/html')) {
    for (const link of LINKS) res.headers.append('Link', link);
  }
  return res;
});
