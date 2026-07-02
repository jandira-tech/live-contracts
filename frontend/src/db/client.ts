import { drizzle, type DrizzleD1Database } from 'drizzle-orm/d1';
import { drizzle as drizzleLocal } from 'drizzle-orm/better-sqlite3';
import Database from 'better-sqlite3';
import * as schema from './schema';

export type DB = DrizzleD1Database<typeof schema> | ReturnType<typeof drizzleLocal>;

let _db: DB | null = null;

// Singleton handle over the D1 binding or local SQLite. In production (Cloudflare
// Workers), uses the D1 binding. In local dev, falls back to better-sqlite3.
export function getDb(): DB {
  if (_db) return _db;

  // Try Cloudflare D1 binding first
  try {
    // @ts-ignore - env may not exist in local dev
    const { env } = require('cloudflare:workers');
    if (env && (env as Env).DB) {
      _db = drizzle((env as Env).DB, { schema });
      return _db;
    }
  } catch {
    // cloudflare:workers not available, fall back to local SQLite
  }

  // Local dev fallback: use better-sqlite3
  const localDb = new Database('./.local-db.sqlite');
  _db = drizzleLocal(localDb, { schema });
  return _db;
}

// Test-only: clear the cached singleton between test files.
export function _resetDb(): void { 
  if (_db) {
    // @ts-ignore - close() exists on better-sqlite3
    _db?.client?.close?.();
  }
  _db = null; 
}
