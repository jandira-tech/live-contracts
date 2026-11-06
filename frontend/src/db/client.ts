import { drizzle, type DrizzleD1Database } from 'drizzle-orm/d1';
import { env } from 'cloudflare:workers';
import * as schema from './schema';

export type DB = DrizzleD1Database<typeof schema>;

let _db: DB | null = null;

// Singleton handle over the D1 binding. The Cloudflare adapter provides `DB` in
// production AND in `astro dev` (a local miniflare database under .wrangler/state;
// seed it with `bun run db:setup`). There is no other backend.
export function getDb(): DB {
  if (_db) return _db;
  const d1 = (env as unknown as Env | undefined)?.DB;
  if (!d1) throw new Error('D1 binding `DB` is missing: check wrangler.jsonc (and run `bun run db:setup` locally).');
  _db = drizzle(d1, { schema });
  return _db;
}

// Test-only: clear the cached singleton between test files.
export function _resetDb(): void {
  _db = null;
}
