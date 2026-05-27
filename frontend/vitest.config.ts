import { defineWorkersConfig } from '@cloudflare/vitest-pool-workers/config';

export default defineWorkersConfig({
  test: {
    poolOptions: {
      workers: {
        miniflare: { d1Databases: { DB: 'sec-ex10-test' } },
        wrangler: { configPath: './wrangler.jsonc' },
      },
    },
  },
});
