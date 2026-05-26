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
      // Internal FastAPI origin — reachable only from the Worker (private network
      // / Cloudflare Tunnel). Never exposed publicly.
      SEC_API_URL: envField.string({
        context: 'server',
        access: 'secret',
        default: 'https://arthrod-sec-ex10-api.hf.space',
      }),
      SEC_API_KEY: envField.string({
        context: 'server',
        access: 'secret',
        optional: true,
      }),
    },
  },
});
