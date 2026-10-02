import type { Ex10Summary, Ex10Detail } from './api';

// Markdown content negotiation: when an agent sends `Accept: text/markdown`, the
// pages return a markdown representation instead of HTML (HTML stays the default
// for browsers). See https://developers.cloudflare.com/fundamentals/reference/markdown-for-agents/.

const SITE = 'https://live-contracts.arthur.law';

export function prefersMarkdown(accept: string | null | undefined): boolean {
  if (!accept) return false;
  // Agents opt in explicitly; browsers never send text/markdown.
  return /\btext\/(x-)?markdown\b/i.test(accept);
}

// Rough token estimate (~4 chars/token) for the x-markdown-tokens header.
function estimateTokens(s: string): number {
  return Math.max(1, Math.ceil(s.length / 4));
}

export function markdownResponse(md: string, status = 200): Response {
  return new Response(md, {
    status,
    headers: {
      'content-type': 'text/markdown; charset=utf-8',
      'x-markdown-tokens': String(estimateTokens(md)),
      vary: 'Accept',
      'cache-control': 'public, max-age=60, stale-while-revalidate=600',
    },
  });
}

function fmtFiled(v: string | undefined): string {
  if (!v) return '';
  const d = v.replace(/\D/g, '');
  return d.length >= 8 ? `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}` : v;
}

function metaLine(it: Ex10Summary): string {
  return [it.company_name || '', it.form_type, it.doc_type, fmtFiled(it.filed_at) && `filed ${fmtFiled(it.filed_at)}`]
    .filter(Boolean)
    .join(' · ');
}

export function listMarkdown(title: string, intro: string, items: Ex10Summary[]): string {
  const out = [`# ${title}`, '', intro, ''];
  if (!items.length) out.push('_No agreements._');
  for (const it of items) {
    const heading = (it.description || `${it.doc_type} — ${it.filename}`).trim();
    out.push(`## ${heading}`);
    out.push(`${metaLine(it)}  `);
    out.push(`CIK ${it.cik} · ${it.accession}  `);
    out.push(`[View on Live Contracts](${SITE}/agreement/${it.id})${it.filing_url ? ` · [SEC filing](${it.filing_url})` : ''}`);
    if (it.excerpt) {
      out.push('');
      out.push(it.excerpt.replace(/\s*\n\s*/g, ' ').trim());
    }
    out.push('');
  }
  out.push('---', '_Public SEC EX-10 filing data. Not legal or investment advice._');
  return out.join('\n');
}

export function detailMarkdown(d: Ex10Detail): string {
  const title = (d.description || `${d.doc_type} — ${d.filename}`).trim();
  const meta = [
    d.company_name && `**Filer:** ${d.company_name}`,
    `**Form:** ${d.form_type}`,
    `**Exhibit:** ${d.doc_type}`,
    `**CIK:** ${d.cik}`,
    `**Accession:** ${d.accession}`,
    fmtFiled(d.filed_at) && `**Filed:** ${fmtFiled(d.filed_at)}`,
  ].filter(Boolean);
  const out = [`# ${title}`, '', meta.join('  \n'), ''];
  out.push(`[Original SEC filing](${d.filing_url}) · [Live Contracts page](${SITE}/agreement/${d.id})`, '', '---', '');
  if (d.markdown) out.push(d.markdown);
  else if (d.image_urls?.length)
    out.push(`_Scanned exhibit — ${d.image_urls.length} page(s)._`, '', ...d.image_urls.map((u, i) => `![Page ${i + 1}](${u})`));
  else out.push('_Markdown for this exhibit is still being generated._');
  return out.join('\n');
}
