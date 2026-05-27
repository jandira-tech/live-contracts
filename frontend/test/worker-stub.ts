// Minimal Worker entrypoint for the vitest-pool-workers harness. The real
// Worker entry (@astrojs/cloudflare/entrypoints/server, from wrangler.jsonc)
// is a package path that the test pool can't statically resolve; our unit
// tests exercise lib/api.ts functions directly with an injected `db`, so they
// never dispatch a fetch. This stub just satisfies the pool's `main` resolver.
export default {
  async fetch(): Promise<Response> {
    return new Response('test stub');
  },
};
