import type { APIRoute } from 'astro';
import { env } from 'cloudflare:workers';
import { ex10Search, ex10Detail, listEx10, type Ex10Summary } from '../lib/api';

// Public MCP server for the Live Contracts SEC EX-10 feed. Stateless Streamable
// HTTP transport: POST a JSON-RPC message, get a JSON response (no sessions, no
// server-initiated SSE). Read-only; reuses the same D1 data layer as the pages.
// Rate-limited per client IP via the GA Workers rate-limit binding.
export const prerender = false;

const SITE = 'https://live-contracts.arthur.law';
const PROTOCOL_VERSION = '2025-06-18';
const SERVER_INFO = { name: 'live-contracts', title: 'Live Contracts — SEC EX-10', version: '1.0.0' };
const INSTRUCTIONS =
  'Read-only access to a real-time feed of EX-10 material-contract exhibits extracted from SEC EDGAR. ' +
  'Use search_agreements for full-text search, browse_agreements to filter by form/filer/CIK, ' +
  'list_recent_agreements for the newest filings, and get_agreement to fetch one with its full text. ' +
  'Data is public SEC filing information — not legal or investment advice.';

const TOOLS = [
  {
    name: 'search_agreements',
    description: 'Full-text search across EX-10 material-contract exhibits (matches description and extracted body). Returns matching agreements with links.',
    inputSchema: {
      type: 'object',
      properties: {
        query: { type: 'string', description: 'Search terms (parties, clauses, keywords).' },
        limit: { type: 'integer', minimum: 1, maximum: 50, default: 20 },
      },
      required: ['query'],
    },
  },
  {
    name: 'list_recent_agreements',
    description: 'List the most recently filed EX-10 agreements, newest first.',
    inputSchema: {
      type: 'object',
      properties: { limit: { type: 'integer', minimum: 1, maximum: 50, default: 20 } },
    },
  },
  {
    name: 'browse_agreements',
    description: 'Browse and filter EX-10 agreements by SEC form type, filer company name, or CIK, with pagination.',
    inputSchema: {
      type: 'object',
      properties: {
        page: { type: 'integer', minimum: 1, default: 1 },
        limit: { type: 'integer', minimum: 1, maximum: 50, default: 20 },
        form: { type: 'string', description: 'SEC form type, e.g. 8-K, 10-K, S-1.' },
        cik: { type: 'string', description: 'SEC Central Index Key of the filer.' },
        filer: { type: 'string', description: 'Filer company name (substring match).' },
        sort: { type: 'string', enum: ['newest', 'oldest'], default: 'newest' },
      },
    },
  },
  {
    name: 'get_agreement',
    description: 'Fetch a single EX-10 agreement by id, including its full extracted markdown text and filing metadata.',
    inputSchema: {
      type: 'object',
      properties: { id: { type: 'string', description: 'Agreement id (UUID).' } },
      required: ['id'],
    },
  },
];

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'POST, OPTIONS',
  'Access-Control-Allow-Headers': 'content-type, mcp-session-id, mcp-protocol-version',
  'Access-Control-Max-Age': '86400',
};

type Json = Record<string, unknown>;
const ok = (id: unknown, result: Json): Json => ({ jsonrpc: '2.0', id: id ?? null, result });
const err = (id: unknown, code: number, message: string): Json => ({ jsonrpc: '2.0', id: id ?? null, error: { code, message } });

function clampInt(v: unknown, min: number, max: number, def: number): number {
  const n = Number(v);
  return Number.isFinite(n) ? Math.min(max, Math.max(min, Math.trunc(n))) : def;
}
const str = (v: unknown): string => (v == null ? '' : String(v));

function trim(it: Ex10Summary): Json {
  return {
    id: it.id,
    url: `${SITE}/agreement/${it.id}`,
    company: it.company_name || '',
    form_type: it.form_type,
    doc_type: it.doc_type,
    description: it.description,
    filed_at: it.filed_at || '',
    cik: it.cik,
    accession: it.accession,
    filing_url: it.filing_url,
    excerpt: it.excerpt,
  };
}

async function callTool(name: string, args: Json): Promise<Json | null> {
  switch (name) {
    case 'search_agreements': {
      const query = str(args.query).trim();
      if (!query) throw new Error('query is required');
      const r = await ex10Search(query, 1, clampInt(args.limit, 1, 50, 20));
      return { total: r.total, count: r.items.length, items: r.items.map(trim) };
    }
    case 'list_recent_agreements': {
      const r = await listEx10(1, clampInt(args.limit, 1, 50, 20), { sort: 'newest' });
      return { total: r.total, count: r.items.length, items: r.items.map(trim) };
    }
    case 'browse_agreements': {
      const r = await listEx10(clampInt(args.page, 1, 100000, 1), clampInt(args.limit, 1, 50, 20), {
        form: str(args.form) || undefined,
        cik: str(args.cik) || undefined,
        filer: str(args.filer) || undefined,
        sort: args.sort === 'oldest' ? 'oldest' : 'newest',
      });
      return { total: r.total, page: r.page, total_pages: r.total_pages, count: r.items.length, items: r.items.map(trim) };
    }
    case 'get_agreement': {
      const id = str(args.id).trim();
      if (!id) throw new Error('id is required');
      const d = await ex10Detail(id);
      if (!d) return null;
      return { ...trim(d), markdown: d.markdown ?? '', filing: (d.filing as Json) ?? {}, image_urls: d.image_urls ?? [] };
    }
    default:
      throw new Error(`unknown tool: ${name}`);
  }
}

// Returns a JSON-RPC response object, or null for notifications (no reply).
async function handle(msg: Json): Promise<Json | null> {
  const { id, method, params } = msg as { id?: unknown; method?: string; params?: Json };
  if (!method) return err(id, -32600, 'Invalid Request');
  if (method === 'initialize')
    return ok(id, { protocolVersion: PROTOCOL_VERSION, capabilities: { tools: { listChanged: false } }, serverInfo: SERVER_INFO, instructions: INSTRUCTIONS });
  if (method.startsWith('notifications/')) return null; // notifications get no response
  if (method === 'ping') return ok(id, {});
  if (method === 'tools/list') return ok(id, { tools: TOOLS });
  if (method === 'tools/call') {
    const name = str((params as Json)?.name);
    const args = ((params as Json)?.arguments as Json) ?? {};
    try {
      const data = await callTool(name, args);
      if (data === null) return ok(id, { content: [{ type: 'text', text: 'No agreement found for that id.' }], isError: true });
      return ok(id, { content: [{ type: 'text', text: JSON.stringify(data, null, 2) }] });
    } catch (e) {
      return ok(id, { content: [{ type: 'text', text: `Error: ${(e as Error)?.message ?? String(e)}` }], isError: true });
    }
  }
  return err(id, -32601, `Method not found: ${method}`);
}

const json = (body: unknown, status = 200, extra: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...CORS, ...extra } });

export const OPTIONS: APIRoute = () => new Response(null, { status: 204, headers: CORS });

export const GET: APIRoute = () =>
  new Response('Live Contracts MCP server. POST JSON-RPC (MCP Streamable HTTP) to this endpoint.', {
    status: 405,
    headers: { Allow: 'POST, OPTIONS', ...CORS },
  });

export const POST: APIRoute = async (ctx) => {
  // Rate limit per client IP (degrades gracefully if the binding is absent).
  const limiter = (env as Env).MCP_RATE_LIMITER;
  if (limiter) {
    const key = ctx.request.headers.get('cf-connecting-ip') ?? 'anon';
    const { success } = await limiter.limit({ key });
    if (!success) return json(err(null, -32029, 'Rate limit exceeded — slow down and retry shortly.'), 429, { 'retry-after': '10' });
  }

  let msg: unknown;
  try {
    msg = await ctx.request.json();
  } catch {
    return json(err(null, -32700, 'Parse error'), 400);
  }

  if (Array.isArray(msg)) {
    const out = (await Promise.all(msg.map((m) => handle(m as Json)))).filter((r): r is Json => r !== null);
    return out.length ? json(out) : new Response(null, { status: 202, headers: CORS });
  }
  const res = await handle(msg as Json);
  return res === null ? new Response(null, { status: 202, headers: CORS }) : json(res);
};
