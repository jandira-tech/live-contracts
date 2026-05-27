import { env } from 'cloudflare:workers';
import { drizzle, type DrizzleD1Database } from 'drizzle-orm/d1';
import * as schema from './schema';

export type DB = DrizzleD1Database<typeof schema>;

let _db: DB | null = null;

// Singleton handle over the D1 binding. `env` from cloudflare:workers resolves
// inside the Worker at runtime (pages, lib/, live loaders). No request scoping
// needed — writes come from a separate origin and reads are CDN-cached.
export function getDb(): DB {
  if (!_db) _db = drizzle((env as Env).DB, { schema });
  return _db;
}

// Test-only: clear the cached singleton between test files.
export function _resetDb(): void { _db = null; }
