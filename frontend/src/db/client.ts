import { drizzle, type DrizzleD1Database } from 'drizzle-orm/d1';
import { drizzle as drizzleLocal } from 'drizzle-orm/better-sqlite3';
import Database from 'better-sqlite3';
import { env } from 'cloudflare:workers';
import * as schema from './schema';

export type DB = DrizzleD1Database<typeof schema> | ReturnType<typeof drizzleLocal>;

let _db: DB | null = null;

// Singleton handle over the D1 binding or local SQLite. In production (Cloudflare
// Workers), uses the D1 binding. In local dev, falls back to better-sqlite3.
export function getDb(): DB {
  if (_db) return _db;

  // Use D1 binding if env.DB exists
  if (env?.DB) {
    _db = drizzle((env as Env).DB, { schema });
    return _db;
  }

  // Local dev fallback: use better-sqlite3
  const localDb = new Database('./.local-db.sqlite');
  _db = drizzleLocal(localDb, { schema });
  return _db;
}

// Test-only: clear the cached singleton between test files.
export function _resetDb(): void {
  if (_db) {
    // Close the underlying SQLite connection on better-sqlite3 instances
    // @ts-ignore - $client exists on drizzle better-sqlite3 instances
    (_db as any)?.$client?.close?.();
  }
  _db = null;
}
