// @cloudflare/vitest-pool-workers@0.16 replaced the old `defineWorkersConfig`
// (from `/config`) with a `cloudflareTest()` Vite plugin used alongside
// Vitest's own `defineConfig`. The `DB` D1 binding is read from wrangler.jsonc;
// the miniflare override names the test database so it gets a local instance.
import { cloudflareTest } from '@cloudflare/vitest-pool-workers';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [
    cloudflareTest({
      // Override the wrangler `main` (a package path the pool can't resolve) with
      // a local stub — unit tests call lib/api fns directly, never dispatch fetch.
      main: './test/worker-stub.ts',
      // SEC_API_KEY: the ingest route reads it from `cloudflare:workers` env
      // (Astro v6 removed locals.runtime.env), so the auth tests need it here.
      // PII_REDACTIONS: fictional patterns; the real list lives only in the Worker secret.
      miniflare: {
        d1Databases: { DB: 'sec-ex10-test' },
        bindings: {
          SEC_API_KEY: 'secret',
          PII_REDACTIONS: JSON.stringify(['Examplestra(?:ß|ss)e\\s*12(?!\\d)', '12345\\s*Ex(?:ä|ae|a)mpletown']),
        },
      },
      wrangler: { configPath: './wrangler.jsonc' },
    }),
  ],
});
