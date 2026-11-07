/**
 * Full-text search over the external-content FTS5 index (migration 0003).
 *
 * Two statements in one D1 batch:
 *  1. an exact total, answered from the index alone;
 *  2. the page: a subquery picks the page's rowids by filed_at, and only those
 *     rows get `snippet()`. Calling snippet() on every hit before the sort would
 *     read and re-tokenize most of the 1.8 GB corpus for a broad term.
 *
 * There is deliberately no substring-scan fallback: a missing or broken index throws, and
 * the callers surface that error instead of scanning `exhibits.markdown`.
 */
import { count, desc, sql } from 'drizzle-orm';
import { exhibits } from '../db/schema';
import { getDb, type DB } from '../db/client';
import { rowToSummary, ftsQuery } from './summary';
import { summaryCols, pages } from './columns';
import type { PageResult } from './api';

export interface SearchResult extends PageResult {
  query: string;
  engine: 'fts';
}

const SNIPPET = sql<string>`snippet(exhibits_fts, 1, '<mark>', '</mark>', '…', 28)`;
// Ties on filed_at are common (one filing carries several EX-10s); the rowid
// tie-break keeps page boundaries stable. idx_ex_filed_at already ends in rowid.
const BY_NEWEST = sql`filed_at DESC, rowid DESC`;

function countQuery(db: DB, match: string) {
  return db
    .select({ n: count() })
    .from(sql`exhibits_fts`)
    .where(sql`exhibits_fts MATCH ${match}`);
}

function pageQuery(db: DB, match: string, limit: number, offset: number) {
  const pageRowids = sql`SELECT rowid FROM exhibits WHERE rowid IN (SELECT rowid FROM exhibits_fts WHERE exhibits_fts MATCH ${match}) ORDER BY ${BY_NEWEST} LIMIT ${limit} OFFSET ${offset}`;
  return db
    .select({ ...summaryCols, snippet: SNIPPET })
    .from(sql`exhibits_fts`)
    .innerJoin(exhibits, sql`${exhibits}.rowid = exhibits_fts.rowid`)
    .where(sql`exhibits_fts MATCH ${match} AND exhibits_fts.rowid IN (${pageRowids})`)
    .orderBy(desc(exhibits.filedAt), sql`${exhibits}.rowid DESC`);
}

/** The page statement's SQL text. Tests pin its shape so snippet() stays page-scoped. */
export function searchPageSql(db: DB, match: string, limit: number, offset: number): string {
  return pageQuery(db, match, limit, offset).toSQL().sql;
}

/**
 * Search descriptions and markdown bodies. Every user token is quoted by
 * `ftsQuery`, so FTS operators in the input are literal; quoted phrases stay
 * phrases. Results are newest first with `<mark>`-wrapped snippets.
 */
export async function ex10Search(query: string, page = 1, pageSize = 20, db: DB = getDb()): Promise<SearchResult> {
  const q = (query ?? '').trim();
  const match = ftsQuery(q);
  if (!match) return { query: q, engine: 'fts', items: [], total: 0, page, page_size: pageSize, total_pages: 0 };
  const [totals, rows] = await db.batch([countQuery(db, match), pageQuery(db, match, pageSize, (page - 1) * pageSize)]);
  const total = totals[0]?.n ?? 0;
  return {
    query: q,
    engine: 'fts',
    items: rows.map((r) => ({ ...rowToSummary(r), snippet: r.snippet ?? '' })),
    total,
    page,
    page_size: pageSize,
    total_pages: pages(total, pageSize),
  };
}
