/**
 * Typed data layer for the SEC EX-10 collection, backed by Cloudflare D1
 * (Drizzle ORM over the `DB` binding). Each function takes an optional `db`
 * as its LAST argument, defaulting to the request-time singleton from
 * `getDb()` — so the live loaders/pages call them unchanged, while tests
 * inject a seeded test database.
 */
import { and, asc, count, desc, eq, sql } from 'drizzle-orm';
import { exhibits } from '../db/schema';
import { getDb, type DB } from '../db/client';
import { rowToSummary, parseFiling, parseImageUrls } from './summary';

export interface FilingHeader {
  company_name?: string;
  cik?: string;
  sic?: string;
  state_of_incorporation?: string;
  period?: string;
  filing_date?: string;
  file_number?: string;
  location?: string;
  items?: string[];
}

export interface Ex10Summary {
  id: string;
  accession: string;
  cik: string;
  form_type: string;
  doc_type: string;
  filename: string;
  description: string;
  filing_url: string;
  found_at: string;
  markdown_status: string;
  excerpt: string;
  has_markdown: boolean;
  // Compact filing-header fields for card footers.
  company_name?: string;
  period?: string;
  location?: string;
  items?: string[];
  filed_at?: string; // SEC acceptance datetime "YYYYMMDDHHMMSS" (ET); "" until backfilled
  image_urls?: string[]; // HF-dataset URLs for scanned (image-only) exhibits; [] otherwise
}

export interface Ex10Detail extends Ex10Summary {
  markdown: string;
  sequence: string;
  filing?: FilingHeader;
}

export interface PageResult {
  items: Ex10Summary[];
  total: number;
  page: number;
  page_size: number;
  total_pages: number;
}

export interface BrowseFilters {
  form?: string;
  cik?: string;
  filer?: string;
  sort?: 'newest' | 'oldest';
}

export interface FormFacet { form_type: string; count: number; }

export interface Stats {
  total: number;
  with_markdown: number;
  pending_markdown: number;
  last_24h: number;
  by_doc_type: Record<string, number>;
  by_form_type: Record<string, number>;
}

export interface SearchResult extends PageResult {
  query: string;
}

// substr keeps list payloads light (markdown bodies avg ~67KB).
const summaryCols = {
  id: exhibits.id, accession: exhibits.accession, cik: exhibits.cik,
  formType: exhibits.formType, docType: exhibits.docType, filename: exhibits.filename,
  description: exhibits.description, sequence: exhibits.sequence, filingUrl: exhibits.filingUrl,
  foundAt: exhibits.foundAt, filedAt: exhibits.filedAt, markdownStatus: exhibits.markdownStatus,
  filingMetadata: exhibits.filingMetadata, imageUrls: exhibits.imageUrls,
  markdown: sql<string>`substr(${exhibits.markdown}, 1, 2000)`,
};
// NULLS LAST emulation (D1's default null ordering is not guaranteed).
const nullsLast = sql`(${exhibits.filedAt} IS NULL OR ${exhibits.filedAt} = '') ASC`;
const orderNewest = [nullsLast, desc(exhibits.filedAt), desc(exhibits.foundAt), desc(exhibits.id)];
const orderOldest = [nullsLast, asc(exhibits.filedAt), asc(exhibits.foundAt), asc(exhibits.id)];

function pages(total: number, size: number) { return total ? Math.max(1, Math.ceil(total / size)) : 0; }

export async function listEx10(page = 1, pageSize = 20, filters: BrowseFilters = {}, db: DB = getDb()): Promise<PageResult> {
  const conds = [];
  if (filters.form) conds.push(eq(exhibits.formType, filters.form));
  if (filters.cik) conds.push(eq(exhibits.cik, filters.cik));
  if (filters.filer) conds.push(sql`json_extract(${exhibits.filingMetadata}, '$.company_name') LIKE ${'%' + filters.filer + '%'} COLLATE NOCASE`);
  const where = conds.length ? and(...conds) : undefined;
  const total = (await db.select({ n: count() }).from(exhibits).where(where))[0]?.n ?? 0;
  const rows = await db.select(summaryCols).from(exhibits).where(where)
    .orderBy(...(filters.sort === 'oldest' ? orderOldest : orderNewest))
    .limit(pageSize).offset((page - 1) * pageSize);
  return { items: rows.map(rowToSummary), total, page, page_size: pageSize, total_pages: pages(total, pageSize) };
}

export async function ex10Facets(db: DB = getDb()): Promise<{ forms: FormFacet[] }> {
  const rows = await db.select({ form_type: exhibits.formType, count: count() }).from(exhibits)
    .where(sql`${exhibits.formType} IS NOT NULL AND ${exhibits.formType} <> ''`)
    .groupBy(exhibits.formType).orderBy(desc(count()), asc(exhibits.formType));
  return { forms: rows as FormFacet[] };
}

export async function ex10Since(seconds = 60, db: DB = getDb()): Promise<{ window_seconds: number; count: number; items: Ex10Summary[] }> {
  const rows = await db.select(summaryCols).from(exhibits)
    .where(sql`${exhibits.foundAt} >= datetime('now', ${'-' + Math.trunc(seconds) + ' seconds'})`)
    .orderBy(...orderNewest);
  const items = rows.map(rowToSummary);
  return { window_seconds: seconds, count: items.length, items };
}

export async function ex10Detail(id: string, db: DB = getDb()): Promise<Ex10Detail | null> {
  const row = (await db.select().from(exhibits).where(eq(exhibits.id, String(id))).limit(1))[0];
  if (!row) return null;
  return { ...rowToSummary(row), sequence: row.sequence ?? '', markdown: row.markdown ?? '',
           filing: parseFiling(row.filingMetadata) as FilingHeader, image_urls: parseImageUrls(row.imageUrls) };
}

export async function ex10Search(query: string, page = 1, pageSize = 20, db: DB = getDb()): Promise<SearchResult> {
  const q = (query ?? '').trim();
  if (!q) return { query: '', items: [], total: 0, page, page_size: pageSize, total_pages: 0 };
  const like = `%${q.replace(/\\/g, '\\\\').replace(/%/g, '\\%').replace(/_/g, '\\_')}%`;
  const cond = sql`(${exhibits.description} LIKE ${like} ESCAPE '\\' COLLATE NOCASE OR ${exhibits.markdown} LIKE ${like} ESCAPE '\\' COLLATE NOCASE)`;
  const total = (await db.select({ n: count() }).from(exhibits).where(cond))[0]?.n ?? 0;
  const rows = await db.select(summaryCols).from(exhibits).where(cond).orderBy(...orderNewest)
    .limit(pageSize).offset((page - 1) * pageSize);
  return { query: q, items: rows.map(rowToSummary), total, page, page_size: pageSize, total_pages: pages(total, pageSize) };
}

export async function ex10Stats(db: DB = getDb()): Promise<Stats> {
  const c = async (w?: ReturnType<typeof sql>) => (await db.select({ n: count() }).from(exhibits).where(w))[0]?.n ?? 0;
  const total = await c();
  const with_markdown = await c(eq(exhibits.markdownStatus, 'done'));
  const pending_markdown = await c(sql`${exhibits.markdownStatus} IS NULL OR ${exhibits.markdownStatus} = 'pending'`);
  const last_24h = await c(sql`${exhibits.foundAt} >= datetime('now','-1 day')`);
  const byForm = await db.select({ k: exhibits.formType, c: count() }).from(exhibits).groupBy(exhibits.formType);
  const byDoc = await db.select({ k: exhibits.docType, c: count() }).from(exhibits).groupBy(exhibits.docType);
  return { total, with_markdown, pending_markdown, last_24h,
           by_form_type: Object.fromEntries(byForm.map((r) => [r.k ?? '', r.c])),
           by_doc_type: Object.fromEntries(byDoc.map((r) => [r.k ?? '', r.c])) };
}
