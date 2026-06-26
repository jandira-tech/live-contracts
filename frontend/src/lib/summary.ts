import type { Ex10Summary } from './api';
import type { ExhibitRow } from '../db/schema';

const IMG_REF = /\([^()]*\.(?:jpe?g|png|gif|tiff?|svg|webp)(?:\s+"[^"]*")?[^()]*\)/gi;
const MD_MARKERS = /\*+|_{2,}|`+|#+|>+|\[|\]|!\[/g;
const LEADING_LABEL = /^\s*(?:(?:exhibit|ex)[\s.\-]*\d+(?:\.\d+)?\b\s*)+/i;

export function cleanExcerpt(text: string | null | undefined, limit = 1000): string {
  if (!text) return '';
  let s = text.replace(/\|/g, ' ').replace(IMG_REF, ' ').replace(MD_MARKERS, '');
  s = s.replace(/-{2,}/g, ' ').replace(/[ \t]+/g, ' ').replace(/[ \t]*\n[ \t]*/g, '\n');
  s = s.replace(/\n{2,}/g, '\n').trim().replace(LEADING_LABEL, '').trim();
  if (s.length <= limit) return s;
  const cut = s.slice(0, limit);
  // Prefer ending on a clean boundary: a sentence/line end in the latter part of
  // the excerpt, else the last word break — so the cut never chops mid-word and
  // the trailing … reads as "there's more" rather than a hard slice.
  const sentence = Math.max(cut.lastIndexOf('. '), cut.lastIndexOf('.\n'), cut.lastIndexOf('; '));
  const word = Math.max(cut.lastIndexOf(' '), cut.lastIndexOf('\n'));
  const b = sentence > limit * 0.6 ? sentence + 1 : word;
  return (b > 0 ? cut.slice(0, b) : cut).trimEnd() + '…';
}

// Real EX-10 markdown often leads with the source filename (e.g. "aspi\_ex101.htm").
// Drop just that token — nothing else — so the snippet and the detail page render
// the same clean markdown through marked.
const FILENAME_HEAD = /^\s*\S+\.(?:html?|txt)\b[\s\\]*/i;
// EDGARMaster prepends a one-line SEC SGML header — "<TYPE> <SEQUENCE> <FILENAME>
// <DESCRIPTION>" (e.g. "EX-10.1 3 aspi_ex101.htm SUBSCRIPTION AGREEMENT") — plus an
// HTML comment, before the exhibit body. None of it is document content.
const SEC_HEADER = /^﻿?[ \t]*(?:EX-[\w.\-]+[ \t]+\d+[ \t]+\S+\.(?:html?|txt)\b[^\n]*\n+)?(?:<!--[\s\S]*?-->\s*)*/i;
export function stripMdHead(md: string | null | undefined): string {
  if (!md) return '';
  return md.replace(SEC_HEADER, '').replace(FILENAME_HEAD, '').trimStart();
}

// Noise in a card preview: markdown image refs (relative SEC URLs → broken
// anyway) and the redundant leading "Exhibit 10.x" label (the whole site is
// EX-10). Stripped from the *snippet* only; the detail page keeps the full doc.
const MD_IMAGE = /!\[[^\]]*\]\([^)]*\)/g;
const LEADING_EXHIBIT = /^[ \t]*(?:#{1,6}[ \t]*)?\**[ \t]*(?:exhibit|ex)\b[ \t.\-]*\d+(?:\.\d+)?[ \t]*\**[ \t]*(?:\r?\n+|$)/i;

// A *markdown* excerpt for the visual cards: a leading slice of the real markdown
// (markers intact), cleaned of the noise above and cut on a block boundary so
// marked never sees a half-open marker/table. Rendered by marked, same as the
// detail page — no hand-rolled clause splitting or quote-bolding.
export function markdownExcerpt(md: string | null | undefined, limit = 1100): string {
  let s = stripMdHead(md);
  if (!s) return '';
  s = s.replace(MD_IMAGE, '').replace(/\[\]\([^)]*\)/g, ''); // images + emptied links
  for (let i = 0; i < 3 && LEADING_EXHIBIT.test(s); i++) s = s.replace(LEADING_EXHIBIT, '');
  s = s.replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim(); // collapse excessive newlines
  if (!s) return '';
  if (s.length <= limit) return s;
  const cut = s.slice(0, limit);
  const para = cut.lastIndexOf('\n\n');
  const line = cut.lastIndexOf('\n');
  const b = para > limit * 0.4 ? para : line > limit * 0.4 ? line : limit;
  return s.slice(0, b).trimEnd() + '\n\n…';
}

export function parseFiling(raw: string | null | undefined): Record<string, unknown> {
  if (!raw) return {};
  try { const v = JSON.parse(raw); return v && typeof v === 'object' && !Array.isArray(v) ? v : {}; }
  catch { return {}; }
}
export function parseImageUrls(raw: string | null | undefined): string[] {
  if (!raw) return [];
  try { const v = JSON.parse(raw); return Array.isArray(v) ? v.map(String) : []; }
  catch { return []; }
}

// Row may carry a truncated markdown (list views select substr(...) AS markdown).
export function rowToSummary(r: Partial<ExhibitRow>): Ex10Summary {
  const filing = parseFiling(r.filingMetadata ?? null);
  return {
    id: String(r.id ?? ''), accession: r.accession ?? '', cik: r.cik ?? '',
    form_type: r.formType ?? '', doc_type: r.docType ?? '', filename: r.filename ?? '',
    description: r.description ?? '', filing_url: r.filingUrl ?? '', found_at: r.foundAt ?? '',
    markdown_status: r.markdownStatus ?? '',
    excerpt: cleanExcerpt(r.markdown, 1000), excerpt_md: markdownExcerpt(r.markdown),
    has_markdown: Boolean(r.markdown),
    company_name: String(filing.company_name ?? ''), period: String(filing.period ?? ''),
    location: String(filing.location ?? ''),
    items: Array.isArray(filing.items) ? (filing.items as string[]) : [],
    filed_at: r.filedAt ?? String(filing.filed_at ?? ''),
    image_urls: parseImageUrls(r.imageUrls ?? null),
  };
}
