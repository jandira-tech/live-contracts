import type { Ex10Summary } from './api';
import type { ExhibitRow } from '../db/schema';

const IMG_REF = /\([^()]*\.(?:jpe?g|png|gif|tiff?|svg|webp)(?:\s+"[^"]*")?[^()]*\)/gi;
const MD_MARKERS = /\*+|_{2,}|`+|#+|>+|\[|\]|!\[/g;
const LEADING_LABEL = /^\s*(?:(?:exhibit|ex)[\s.-]*\d+(?:\.\d+)?\b\s*)+/i;

export function cleanExcerpt(text: string | null | undefined, limit = 1000): string {
  if (!text) return '';
  let s = text.replace(/\|/g, ' ').replace(IMG_REF, ' ').replace(MD_MARKERS, '');
  s = s
    .replace(/-{2,}/g, ' ')
    .replace(/[ \t]+/g, ' ')
    .replace(/[ \t]*\n[ \t]*/g, '\n');
  s = s
    .replace(/\n{2,}/g, '\n')
    .trim()
    .replace(LEADING_LABEL, '')
    .trim();
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
const SEC_HEADER = /^﻿?[ \t]*(?:EX-[\w.-]+[ \t]+\d+[ \t]+\S+\.(?:html?|txt)\b[^\n]*\n+)?(?:<!--[\s\S]*?-->\s*)*/i;
export function stripMdHead(md: string | null | undefined): string {
  if (!md) return '';
  return md.replace(SEC_HEADER, '').replace(FILENAME_HEAD, '').trimStart();
}

// Noise in a card preview: markdown image refs (relative SEC URLs → broken
// anyway) and the redundant leading "Exhibit 10.x" label (the whole site is
// EX-10). Stripped from the *snippet* only; the detail page keeps the full doc.
const MD_IMAGE = /!\[[^\]]*\]\([^)]*\)/g;
const LEADING_EXHIBIT =
  /^[ \t]*(?:#{1,6}[ \t]*)?\**[ \t]*(?:exhibit|ex)\b[ \t.-]*\d+(?:\.\d+)?[ \t]*\**[ \t]*(?:\r?\n+|$)/i;

// A *markdown* excerpt for the visual cards: a leading slice of the real markdown
// (markers intact), cleaned of the noise above and cut on a block boundary so
// marked never sees a half-open marker/table. Rendered by marked, same as the
// detail page — no hand-rolled clause splitting or quote-bolding.
export function markdownExcerpt(md: string | null | undefined, limit = 1600): string {
  let s = stripMdHead(md);
  if (!s) return '';
  s = s.replace(MD_IMAGE, '').replace(/\[\]\([^)]*\)/g, ''); // images + emptied links
  for (let i = 0; i < 3 && LEADING_EXHIBIT.test(s); i++) s = s.replace(LEADING_EXHIBIT, '');
  s = s
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim(); // collapse excessive newlines
  if (!s) return '';
  if (s.length <= limit) return s;
  const cut = s.slice(0, limit);
  const para = cut.lastIndexOf('\n\n');
  const line = cut.lastIndexOf('\n');
  // Back off to a block boundary only when it is near the cap; otherwise cut at
  // the last word so the card is not trimmed to a short opening paragraph.
  const word = cut.lastIndexOf(' ');
  const b = para > limit * 0.55 ? para : line > limit * 0.55 ? line : word > limit * 0.55 ? word : limit;
  return s.slice(0, b).trimEnd() + '\n\n…';
}

export function parseFiling(raw: string | null | undefined): Record<string, unknown> {
  if (!raw) return {};
  try {
    const v = JSON.parse(raw);
    return v && typeof v === 'object' && !Array.isArray(v) ? v : {};
  } catch {
    return {};
  }
}
export function parseImageUrls(raw: string | null | undefined): string[] {
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? v.map(String) : [];
  } catch {
    return [];
  }
}

// Filing JSON values are `unknown`. Only primitives have a defined string form;
// an object must not become "[object Object]".
function textOf(v: unknown): string {
  return typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean' ? String(v) : '';
}

// Row may carry a truncated markdown (list views select substr(...) AS markdown).
export function rowToSummary(r: Partial<ExhibitRow>): Ex10Summary {
  const filing = parseFiling(r.filingMetadata ?? null);
  return {
    id: String(r.id ?? ''),
    accession: r.accession ?? '',
    cik: r.cik ?? '',
    form_type: r.formType ?? '',
    doc_type: r.docType ?? '',
    filename: r.filename ?? '',
    description: r.description ?? '',
    filing_url: r.filingUrl ?? '',
    found_at: r.foundAt ?? '',
    markdown_status: r.markdownStatus ?? '',
    excerpt: cleanExcerpt(r.markdown, 1000),
    excerpt_md: markdownExcerpt(r.markdown),
    has_markdown: Boolean(r.markdown),
    company_name: textOf(filing.company_name),
    period: textOf(filing.period),
    location: textOf(filing.location),
    items: Array.isArray(filing.items) ? (filing.items as string[]) : [],
    filed_at: r.filedAt ?? textOf(filing.filed_at),
    image_urls: parseImageUrls(r.imageUrls ?? null),
  };
}

// ---------------------------------------------------------------------------
// Card headline. The SEC `description` is usually just the exhibit label
// ("EX-10.1", "EXHIBIT 10.34"); the real title is the first heading-like line
// of the markdown. Pure functions, unit-tested in test/summary.test.ts.
// ---------------------------------------------------------------------------
const LABEL_ONLY = /^\s*(?:(?:exhibit|ex)[\s.-]*\d+(?:\.\d+)?[a-z]?)?\s*$/i;
export function isLabelOnly(description: string | null | undefined): boolean {
  return LABEL_ONLY.test(description ?? '');
}

const TITLE_WORDS =
  /\b(AGREEMENT|AMENDMENT|NOTE|LEASE|WARRANT|INDENTURE|GUARANTY|GUARANTEE|PLAN|LETTER|CONSENT|WAIVER|RELEASE|CERTIFICATE|LICENSE|SUBLEASE|ASSIGNMENT|JOINDER|SUPPLEMENT|TERMS|CONTRACT|ARRANGEMENT|DEED|UNDERTAKING|OFFER|MEMORANDUM|SCHEDULE)\b/i;
const TITLE_CUT = /\s*(?:\(|“|"|dated|made|entered|is made|by and|between|among|this|has not)\b.*$/i;
const SMALL = new Set(['of', 'and', 'the', 'to', 'for', 'in', 'by', 'or', 'a', 'an', 'as', 'at', 'on', 'with']);
function titleCase(s: string): string {
  return s
    .toLowerCase()
    .split(/\s+/)
    .map((w, i) => {
      if (i > 0 && SMALL.has(w)) return w;
      if (/^\d/.test(w)) return w;
      return w.charAt(0).toUpperCase() + w.slice(1);
    })
    .join(' ');
}

/** Visible text of a markdown slice: no markers, rules, comments or images. */
export function textContent(md: string | null | undefined): string {
  if (!md) return '';
  return (
    md
      .replace(/<!--[\s\S]*?-->/g, ' ')
      .replace(/<[^>]+>/g, ' ')
      .replace(MD_IMAGE, ' ')
      .replace(/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/gm, ' ')
      .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
      // CommonMark: a backslash before a newline is a hard break; one before ASCII
      // punctuation is an escape, so the character is kept even when it is a
      // marker (`aspi\_ex101.htm`). Unescaped markers are dropped in the same pass.
      .replace(/\\(?=\r?\n)/g, ' ')
      .replace(/\\([!-/:-@[-`{-~])|[*_`#>|]+/g, (_m, esc: string | undefined) => esc ?? '')
      .replace(/\s+/g, ' ')
      .trim()
  );
}

const DANGLING = /\b(?:to|of|and|the|for|dated|between|among|&|a|an|by)$/i;
const SENTENCE = /\b(?:is|are|has|was|were|hereby|made|entered|shall|agrees?)\b/i;
const CORP = /\b(?:inc|llc|l\.?p|corp|corporation|company|co|ltd|plc|n\.?a)\.?,?$/i;
const DATE_LINE =
  /^(?:(?:january|february|march|april|may|june|july|august|september|october|november|december)\s+\d{1,2},?\s+\d{4}|\d{1,2}\/\d{1,2}\/\d{2,4})$/i;
const GENERIC = /^(?:agreement|amendment|note|plan|letter|contract|exhibit)$/i;

export function displayTitle(
  description: string | null | undefined,
  markdown: string | null | undefined,
  max = 90,
): string {
  const desc = (description ?? '').trim();
  if (!isLabelOnly(desc)) return desc === desc.toUpperCase() && desc.split(/\s+/).length > 2 ? titleCase(desc) : desc;
  const fallback = desc || 'EX-10';
  // A hard break (`\` before a newline) splits one heading over two lines; drop the
  // backslash here so the per-line pass below can re-join them.
  let body = stripMdHead(markdown)
    .replace(MD_IMAGE, '')
    .replace(/\\(?=\r?\n)/g, '');
  for (let i = 0; i < 3 && LEADING_EXHIBIT.test(body); i++) body = body.replace(LEADING_EXHIBIT, '');
  const raw = body
    .split('\n')
    .map((l) => textContent(l))
    .filter(Boolean)
    .slice(0, 14);
  // Re-join a heading that wrapped after a preposition ("NINTH AMENDMENT TO" / "CREDIT AGREEMENT").
  const lines: string[] = [];
  for (const l of raw) {
    const prev = lines[lines.length - 1];
    if (prev && DANGLING.test(prev) && prev.length + l.length < max) lines[lines.length - 1] = `${prev} ${l}`;
    else lines.push(l);
  }
  const clean = (t: string) => titleCase(t.replace(/[.:;,]+$/, '').trim());
  const standalone = (l: string) =>
    l.length <= max && !DANGLING.test(l) && !SENTENCE.test(l) && !/^(?:this|the)\b/i.test(l);
  // 1) a short standalone line that names the instrument ("CREDIT AGREEMENT").
  for (const l of lines) {
    if (standalone(l) && TITLE_WORDS.test(l) && !GENERIC.test(l)) return clean(l);
  }
  // 2) a letter agreement's subject line.
  for (const l of lines) {
    const m = l.match(/^re:\s*(.{3,})$/i);
    if (m && m[1].length <= max) return `Re: ${clean(m[1])}`;
  }
  // 3) an inline opener ("THIS CONTRACTOR SERVICES AGREEMENT (the ...) is made ...").
  for (const l of lines) {
    const m = l.match(
      /^(?:this\s+)?([A-Z][A-Z0-9 .,&'\-/]{3,}?\b(?:AGREEMENT|NOTE|LEASE|WARRANT|PLAN|LETTER|AMENDMENT|INDENTURE|GUARANTY)\b)/i,
    );
    if (m) {
      const t = m[1].replace(TITLE_CUT, '').trim();
      if (t.length >= 8 && t.length <= max && t === t.toUpperCase() && !GENERIC.test(t) && !DANGLING.test(t))
        return titleCase(t);
    }
  }
  // 4) first heading-like line that is not a sentence, a date, or a party name.
  const first = lines.find(
    (l) =>
      l.length >= 4 &&
      standalone(l) &&
      !/[.!?]\s/.test(l) &&
      !DATE_LINE.test(l) &&
      !CORP.test(l) &&
      !/^dear\b/i.test(l),
  );
  return first ? clean(first) : fallback;
}

// ---------------------------------------------------------------------------
// Image-only exhibits: the markdown carries `![](name.jpg)` refs relative to the
// EDGAR filing folder. Resolve them so the detail page shows the pages instead
// of eighteen broken images.
// ---------------------------------------------------------------------------
const REL_IMG = /!\[([^\]]*)\]\((?!https?:|data:|\/)([^)\s]+)(\s+"[^"]*")?\)/g;
export function resolveEdgarImages(md: string | null | undefined, filingUrl: string | null | undefined): string {
  if (!md) return '';
  const u = (filingUrl ?? '').trim();
  if (!u) return md;
  const base = u.replace(/[?#].*$/, '').replace(/\/[^/]*$/, '/');
  return md.replace(REL_IMG, (_, alt, src, title) => `![${alt}](${base}${src}${title ?? ''})`);
}

// ---------------------------------------------------------------------------
// FTS5 query: every user token is quoted so operators (AND, OR, NOT, *, ^, :)
// are literal; a quoted phrase in the input stays a phrase.
// ---------------------------------------------------------------------------
export function ftsQuery(input: string | null | undefined): string {
  const s = (input ?? '').trim();
  if (!s) return '';
  const parts: string[] = [];
  // A balanced "…" is a phrase; any other quote is a token boundary (#86).
  const re = /"([^"]+)"|([^\s"]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(s))) {
    const raw = (m[1] ?? m[2]).replace(/[*^:()]/g, ' ').trim();
    if (raw) parts.push(`"${raw}"`);
  }
  return parts.join(' ');
}

/** FTS5 snippet() delimiters: private-use characters, so a literal "<mark>" in a filing stays text (#86). */
export const SNIPPET_OPEN = '\uE000';
export const SNIPPET_CLOSE = '\uE001';
const OPEN = SNIPPET_OPEN;
const CLOSE = SNIPPET_CLOSE;

/** Drop a close sentinel with no open one and close one left open, before the end ellipsis. */
function balanceMarks(text: string): string {
  let open = false;
  let out = '';
  for (const ch of text) {
    if (ch === OPEN) {
      if (open) continue;
      open = true;
    } else if (ch === CLOSE) {
      if (!open) continue;
      open = false;
    }
    out += ch;
  }
  if (!open) return out;
  return out.endsWith('…') ? `${out.slice(0, -1).trimEnd()}${CLOSE}…` : `${out.trimEnd()}${CLOSE}`;
}

/**
 * Render an FTS5 snippet (plain markdown text with SNIPPET_OPEN/CLOSE around hits) as safe HTML for a card.
 * The tags are swapped for private-use sentinels first, so stripping markdown markers
 * cannot eat their angle brackets; everything else is escaped. `>` counts as a marker
 * only where it opens a token (blockquote), so a literal `a<b>` survives escaped.
 */
export function snippetHtml(snippet: string | null | undefined): string {
  const tidy = (t: string) =>
    t
      .replace(new RegExp(`${OPEN}\\s*${CLOSE}`, 'g'), ' ')
      .replace(/\s+/g, ' ')
      .trim();
  const plain = tidy(
    (snippet ?? '')
      // Zero-width characters first: one inside `<!--` would hide the comment.
      .replace(/[\u200B-\u200D\uFEFF]/g, '')
      // HTML comments (Workiva/Wdesk banners) and Word downlevel fragments are
      // not text. A snippet can end inside a comment; one that starts with "…"
      // can start inside one, and then only its closing marker is dropped, so a
      // stray "-->" in prose never costs the words before it.
      .replace(/<!--[\s\S]*?-->/g, ' ')
      .replace(/<!\[(?:if[^\]]*|endif)\](?:--)?>/gi, ' ')
      .replace(/^(…(?:(?!<!--)[\s\S])*?)\s*-->/, '$1 ')
      .replace(/\s*<!--[\s\S]*$/, '…')
      .replace(/[#*_`|\\]+/g, '')
      .replace(/(^|\s)>+/g, '$1')
      .replace(/(^|\s)[-:\s]*-{3,}[-:\s]*(?=\s|$)/g, '$1')
      .replace(/(^|\s)-(?=\s|$)/g, '$1'),
  );
  const html = tidy(balanceMarks(plain));
  if (!html.replace(new RegExp(`[…\\s${OPEN}${CLOSE}]`, 'g'), '')) return '';
  return html
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(new RegExp(OPEN, 'g'), '<mark class="sl-mark">')
    .replace(new RegExp(CLOSE, 'g'), '</mark>');
}
