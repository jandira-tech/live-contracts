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
    excerpt: cleanExcerpt(r.markdown, 1000), has_markdown: Boolean(r.markdown),
    company_name: String(filing.company_name ?? ''), period: String(filing.period ?? ''),
    location: String(filing.location ?? ''),
    items: Array.isArray(filing.items) ? (filing.items as string[]) : [],
    filed_at: r.filedAt ?? String(filing.filed_at ?? ''),
    image_urls: parseImageUrls(r.imageUrls ?? null),
  };
}
