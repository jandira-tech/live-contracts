import { marked } from 'marked';

// HTML-to-markdown leaves the trailing space of a run-in heading inside the
// markers (`**1. Engagement. **Subject`). CommonMark will not close emphasis
// when whitespace precedes `**`, so the asterisks show as text. Move that
// whitespace to after the closer.
//
// The body must start with a letter, number, or `$`, stay on one line, and
// contain no `*`, `_`, or `|`. The opener must not follow a letter, number,
// or `*`, and the closer must not be followed by `*`. That keeps table cells,
// adjacent bolds (`**A**, **B**`), and `***italics***` intact. Single `*`
// and `_` are left alone. Fenced and inline code are not rewritten.
const TRAILING_STRONG = /(?<![\p{L}\p{N}*])\*\*([\p{L}\p{N}$][^*\n|_]{0,80}?)[ \t\u00a0]+\*\*(?!\*)/gu;
const CODE_CHUNK = /((?:```|~~~)[\s\S]*?(?:```|~~~)|`+[^`]*`+)/g;

export function closeRunInStrong(md: string): string {
  return md
    .split(CODE_CHUNK)
    .map((chunk, i) => (i % 2 ? chunk : chunk.replace(TRAILING_STRONG, (_m, body: string) => `**${body}** `)))
    .join('');
}

export function renderMarkdown(md: string): string {
  return marked.parse(closeRunInStrong(md)) as string;
}
