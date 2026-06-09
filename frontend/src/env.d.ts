/// <reference path="../.astro/types.d.ts" />

type RateLimit = { limit(options: { key: string }): Promise<{ success: boolean }> };
type Env = {
  DB: import('@cloudflare/workers-types').D1Database;
  SEC_API_KEY?: string;
  // GA Workers rate-limit binding gating the public /mcp endpoint (optional so
  // local/staging without the binding still typecheck and degrade gracefully).
  MCP_RATE_LIMITER?: RateLimit;
};
type Runtime = import('@astrojs/cloudflare').Runtime<Env>;
declare namespace App {
  interface Locals extends Runtime {}
}
