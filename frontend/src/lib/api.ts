/**
 * Typed client for the internal SEC EX-10 API.
 *
 * The API is private (localhost / Cloudflare Tunnel). The Worker holds the key.
 * Every call is defensive: a network error or non-200 yields an empty result
 * instead of throwing, so neither the build (prerender) nor a live request
 * ever hard-fails because the origin blipped.
 */
import { SEC_API_URL, SEC_API_KEY } from 'astro:env/server';

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
  id: number;
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

function headers(): Record<string, string> {
  const h: Record<string, string> = { Accept: 'application/json' };
  if (SEC_API_KEY) h['X-API-Key'] = SEC_API_KEY;
  return h;
}

async function getJson<T>(path: string, fallback: T): Promise<T> {
  try {
    // Bounded timeout so a slow/cold backend can't hang the Worker request;
    // we degrade to the fallback and let the next request (or refresh) retry.
    const res = await fetch(`${SEC_API_URL}${path}`, {
      headers: headers(),
      signal: AbortSignal.timeout(8000),
    });
    if (!res.ok) return fallback;
    return (await res.json()) as T;
  } catch {
    return fallback;
  }
}

export function listEx10(page = 1, pageSize = 20): Promise<PageResult> {
  return getJson<PageResult>(`/api/ex10?page=${page}&page_size=${pageSize}`, {
    items: [],
    total: 0,
    page,
    page_size: pageSize,
    total_pages: 0,
  });
}

export async function ex10Since(seconds = 60): Promise<{ window_seconds: number; count: number; items: Ex10Summary[] }> {
  return getJson(`/api/ex10/since?seconds=${seconds}`, { window_seconds: seconds, count: 0, items: [] });
}

export async function ex10Detail(id: number | string): Promise<Ex10Detail | null> {
  return getJson<Ex10Detail | null>(`/api/ex10/${id}`, null);
}

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

export function ex10Search(query: string, page = 1, pageSize = 20): Promise<SearchResult> {
  const q = encodeURIComponent(query);
  return getJson<SearchResult>(`/api/search?q=${q}&page=${page}&page_size=${pageSize}`, {
    query,
    items: [],
    total: 0,
    page,
    page_size: pageSize,
    total_pages: 0,
  });
}

export function ex10Stats(): Promise<Stats> {
  return getJson<Stats>('/api/stats', {
    total: 0,
    with_markdown: 0,
    pending_markdown: 0,
    last_24h: 0,
    by_doc_type: {},
    by_form_type: {},
  });
}

/** Build-time helper: page through the whole collection for prerendering. */
export async function listAllEx10(pageSize = 100, max = 5000): Promise<Ex10Summary[]> {
  const all: Ex10Summary[] = [];
  let page = 1;
  while (all.length < max) {
    const res = await listEx10(page, pageSize);
    all.push(...res.items);
    if (res.items.length === 0 || page >= res.total_pages) break;
    page += 1;
  }
  return all;
}
