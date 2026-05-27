import { defineConfig, envField } from 'astro/config';
import cloudflare from '@astrojs/cloudflare';

// Astro 6 hybrid: server output by default, individual pages opt into
// prerendering via `export const prerender = true`. Deployed to Cloudflare
// Workers; the CDN caches responses (Cache-Control + stale-while-revalidate set
// per-route and at the API origin).
export default defineConfig({
  site: 'https://live-contracts.arthur.law',
  output: 'server',
  adapter: cloudflare({
    imageService: 'passthrough',
  }),
  env: {
    schema: {
      // Read path is now D1 (Drizzle over the `DB` binding); SEC_API_URL is gone.
      // SEC_API_KEY is kept: PR3's /api/ingest reuses it to gate writes.
      SEC_API_KEY: envField.string({
        context: 'server',
        access: 'secret',
        optional: true,
      }),
    },
  },
});
