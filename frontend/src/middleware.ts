import { defineMiddleware } from 'astro:middleware';

// RFC 8288 Link headers for agent discovery. We advertise the sitemap (an IANA-
// registered relation) on every HTML document response. Static assets (fonts,
// robots.txt) are served by the ASSETS binding and don't pass through here.
export const onRequest = defineMiddleware(async (_context, next) => {
  const res = await next();
  const ct = res.headers.get('content-type') ?? '';
  if (ct.includes('text/html')) {
    res.headers.append('Link', '</sitemap.xml>; rel="sitemap"; type="application/xml"');
  }
  return res;
});
