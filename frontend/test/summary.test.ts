import { describe, it, expect } from 'vitest';
import { stripMdHead, markdownExcerpt, rowToSummary } from '../src/lib/summary';

describe('stripMdHead', () => {
  it('strips a one-line SEC SGML header (TYPE SEQUENCE FILENAME DESCRIPTION)', () => {
    const md = 'EX-10.1 3 aspi_ex101.htm SUBSCRIPTION AGREEMENT\n\nThe parties agree as follows.';
    expect(stripMdHead(md)).toBe('The parties agree as follows.');
  });

  it('strips a leading BOM together with the SEC header', () => {
    const md = '\uFEFFEX-10.5 4 filename.txt Some Description\nActual body content.';
    expect(stripMdHead(md)).toBe('Actual body content.');
  });

  it('strips HTML comments that follow the SEC header', () => {
    const md = 'EX-10.5 4 filename.txt Some Description\n<!-- generated -->\n<!-- another -->\nActual body content.';
    expect(stripMdHead(md)).toBe('Actual body content.');
  });

  it('strips a lone HTML comment when there is no SEC header line', () => {
    const md = '<!-- Document Comment -->\nBody text here.';
    expect(stripMdHead(md)).toBe('Body text here.');
  });

  it('strips a bare leading filename token (with escaped underscore) when there is no full header', () => {
    const md = 'aspi\\_ex101.htm\n\nThe Agreement is dated as of January 1.';
    expect(stripMdHead(md)).toBe('The Agreement is dated as of January 1.');
  });

  it('leaves already-clean markdown untouched', () => {
    expect(stripMdHead('Plain paragraph with no header.')).toBe('Plain paragraph with no header.');
  });

  it('null / undefined / empty → empty string', () => {
    expect(stripMdHead(null)).toBe('');
    expect(stripMdHead(undefined)).toBe('');
    expect(stripMdHead('')).toBe('');
  });
});

describe('markdownExcerpt', () => {
  it('passes through short, clean markdown unchanged', () => {
    expect(markdownExcerpt('Hello world.')).toBe('Hello world.');
  });

  it('strips a SEC header and a leading heading-style Exhibit label', () => {
    const md = 'EX-10.1 3 test.htm DESC\n\nExhibit 10.1\n\nThe parties agree to the following terms.';
    expect(markdownExcerpt(md)).toBe('The parties agree to the following terms.');
  });

  it('strips a bold, inline "Exhibit N.N" label in one pass', () => {
    const md = '**Exhibit 10.3**\n\nBody text.';
    expect(markdownExcerpt(md)).toBe('Body text.');
  });

  it('strips a leading "EX N.N" label without a dot separator', () => {
    const md = 'EX 10.2\n\nBody text after the label.';
    expect(markdownExcerpt(md)).toBe('Body text after the label.');
  });

  it('removes markdown image references entirely', () => {
    const md = '![Company Logo](https://example.com/logo.png)\n\nAgreement body here.';
    expect(markdownExcerpt(md)).toBe('Agreement body here.');
  });

  it('removes emptied markdown links ([]())', () => {
    const md = '[](http://example.com/foo)Some text after.';
    expect(markdownExcerpt(md)).toBe('Some text after.');
  });

  it('collapses 3+ consecutive newlines down to a single blank line', () => {
    const md = 'Intro line.\n\n\n\nSecond paragraph.';
    expect(markdownExcerpt(md)).toBe('Intro line.\n\nSecond paragraph.');
  });

  it('truncates on a paragraph boundary and appends an ellipsis', () => {
    const md = 'A'.repeat(30) + '\n\n' + 'B'.repeat(30);
    expect(markdownExcerpt(md, 50)).toBe('A'.repeat(30) + '\n\n…');
  });

  it('falls back to a hard cut at the limit when there is no usable break', () => {
    const md = 'A'.repeat(2000);
    const out = markdownExcerpt(md, 1100);
    expect(out).toBe('A'.repeat(1100) + '\n\n…');
  });

  it('returns empty string when the entire input is stripped away', () => {
    expect(markdownExcerpt('![img](url)')).toBe('');
  });

  it('null / undefined / empty → empty string', () => {
    expect(markdownExcerpt(null)).toBe('');
    expect(markdownExcerpt(undefined)).toBe('');
    expect(markdownExcerpt('')).toBe('');
  });
});

describe('rowToSummary: excerpt vs excerpt_md', () => {
  it('produces a plaintext excerpt (SEC header intact) and a clean markdown excerpt (SEC header + label stripped)', () => {
    const row = {
      id: '1',
      accession: 'acc-1',
      cik: '111',
      formType: '8-K',
      docType: 'EX-10.1',
      filename: 'test.htm',
      description: 'A test agreement',
      filingUrl: 'https://sec/test',
      foundAt: '2026-01-01T00:00:00',
      filedAt: '20260101000000',
      markdownStatus: 'done',
      filingMetadata: JSON.stringify({ company_name: 'Acme Corp', period: '2026', items: ['1.01'] }),
      imageUrls: '[]',
      markdown: 'EX-10.1 3 test.htm DESC\n\nExhibit 10.1\n\nThe parties agree to the following terms.',
    };

    const summary = rowToSummary(row);

    expect(summary.has_markdown).toBe(true);
    // excerpt (plaintext, for agent/MCP endpoints) does not run through stripMdHead —
    // the raw SEC header survives cleanExcerpt's markdown-marker stripping.
    expect(summary.excerpt).toBe('3 test.htm DESC\nExhibit 10.1\nThe parties agree to the following terms.');
    // excerpt_md (rendered by marked on the cards) strips the SEC header and the
    // redundant "Exhibit 10.1" label, leaving only the real document body.
    expect(summary.excerpt_md).toBe('The parties agree to the following terms.');
    expect(summary.company_name).toBe('Acme Corp');
  });

  it('has_markdown is false and both excerpt fields are empty when markdown is absent', () => {
    const row = {
      id: '2',
      accession: 'acc-2',
      cik: '222',
      formType: '10-Q',
      docType: 'EX-10.2',
      filename: 'b.htm',
      description: '',
      filingUrl: 'https://sec/b',
      foundAt: '2026-01-02T00:00:00',
      filedAt: '',
      markdownStatus: 'pending',
      filingMetadata: null,
      imageUrls: null,
      markdown: null,
    };

    const summary = rowToSummary(row);

    expect(summary.has_markdown).toBe(false);
    expect(summary.excerpt).toBe('');
    expect(summary.excerpt_md).toBe('');
  });
});
// ---------------------------------------------------------------------------
// Card headline: 94 of 100 live rows carry a label-only SEC description
// ("EX-10.1", "EXHIBIT 10.1"); the real title lives in the markdown.
// ---------------------------------------------------------------------------
import {
  displayTitle,
  isLabelOnly,
  textContent,
  resolveEdgarImages,
  ftsQuery,
  snippetHtml,
  SNIPPET_OPEN,
  SNIPPET_CLOSE,
} from '../src/lib/summary';

/** What FTS5 emits now: hits wrapped in the private-use delimiters, not <mark> (#86). */
const fts = (s: string) => s.replace(/<mark>/g, SNIPPET_OPEN).replace(/<\/mark>/g, SNIPPET_CLOSE);

describe('isLabelOnly', () => {
  it('recognises SEC exhibit labels in every spelling', () => {
    for (const s of ['EX-10.1', 'EXHIBIT 10.1', 'Exhibit 10.34', 'ex10.2', 'EX-10', 'EX 10.1', ''])
      expect(isLabelOnly(s)).toBe(true);
  });
  it('keeps a real description', () => {
    expect(isLabelOnly('Material Commercial Agreement')).toBe(false);
    expect(isLabelOnly('EX-10.1 Credit Agreement')).toBe(false);
  });
});

describe('textContent markdown escapes (#84)', () => {
  it('treats a trailing backslash as a line break and unescapes punctuation', () => {
    expect(textContent('AMENDMENT TO\\\nSTOCK PURCHASE')).toBe('AMENDMENT TO STOCK PURCHASE');
    expect(textContent('Price of \\$1,000 \\(net\\) and 5\\% off')).toBe('Price of $1,000 (net) and 5% off');
    expect(textContent('a path C:\\data stays')).toBe('a path C:\\data stays');
  });
});

describe('displayTitle', () => {
  it('keeps a real description untouched', () => {
    expect(displayTitle('Supply Agreement', '# Something else')).toBe('Supply Agreement');
  });
  it('lifts the agreement title out of the markdown when the description is a label', () => {
    const md =
      'Execution Version ANNOVIS BIO, INC. 19,000,000 Shares of Common Stock\n\nUNDERWRITING AGREEMENT\n\nNovember 5, 2026';
    expect(displayTitle('EXHIBIT 10.1', md)).toBe('Underwriting Agreement');
  });
  it('takes the title from the first sentence when it is inline', () => {
    const md =
      '**THIS CONTRACTOR SERVICES AGREEMENT **(“ Agreement ”) is made as of August 14, 2026 BETWEEN: PROFOUND MEDICAL CORP.';
    expect(displayTitle('EX-10.3', md)).toBe('Contractor Services Agreement');
  });
  it('handles amendments and notes', () => {
    expect(
      displayTitle(
        'EX-10.1',
        'EXECUTION VERSION\n\nAMENDMENT NO. 3\n\nDated as of November 5, 2026 to AMENDED AND RESTATED CREDIT AGREEMENT',
      ),
    ).toBe('Amendment No. 3');
    expect(
      displayTitle(
        'EXHIBIT 10.1',
        'PROMISSORY NOTE\n\nTHIS UNSECURED PROMISSORY NOTE (THIS “ NOTE ”) HAS NOT BEEN REGISTERED',
      ),
    ).toBe('Promissory Note');
  });
  it('joins a heading split by a markdown hard break instead of showing the backslash (#84)', () => {
    const md =
      '# EXHIBIT 10.59\n\n---\n\n**Exhibit 10.59**\n\n**AMENDMENT TO\\\nSTOCK PURCHASE AGREEMENT**\n\nThis AMENDMENT (this “**Amendment**”) is made and entered into as of November 4, 2026';
    expect(displayTitle('EXHIBIT 10.59', md)).toBe('Amendment to Stock Purchase Agreement');
  });
  it('skips the repeated exhibit label and falls back to the label when nothing fits', () => {
    expect(
      displayTitle('EX-10.2', 'Exhibit 10.2\n\nMUTUAL SEPARATION AGREEMENT\n\nTHIS MUTUAL SEPARATION AGREEMENT'),
    ).toBe('Mutual Separation Agreement');
    expect(displayTitle('EX-10.1', '---\n<!-- a.jpg -->\n---')).toBe('EX-10.1');
    expect(displayTitle('EX-10.1', '')).toBe('EX-10.1');
  });
  it('joins a heading that wraps after a preposition', () => {
    expect(displayTitle('EX-10.1', 'NINTH AMENDMENT TO\nCREDIT AGREEMENT\n\nThis Ninth Amendment')).toBe(
      'Ninth Amendment to Credit Agreement',
    );
  });
  it('prefers a Re: line in a letter agreement', () => {
    expect(
      displayTitle('EX-10.10', 'September 28, 2026\n\nOliver Reeves\n\nRe: Separation from Employment\n\nDear Oliver:'),
    ).toBe('Re: Separation From Employment');
    expect(
      displayTitle('EX-10.1', 'Vaxart, Inc.\n\nNovember 3, 2026\n\nDear Sam,\n\nThe purpose of this letter is to'),
    ).not.toBe('Vaxart, Inc.');
  });
  it('does not accept a sentence fragment or a bare generic word', () => {
    expect(
      displayTitle('EX-10.1', 'The purpose of\nthis letter is to confirm. Reference is hereby made to the Agreement.'),
    ).toBe('EX-10.1');
    expect(displayTitle('EX-10.2', 'AGREEMENT\n\nTHIS SECURITIES PURCHASE AGREEMENT (this “Agreement”) is dated')).toBe(
      'Securities Purchase Agreement',
    );
  });
  it('title-cases a shouted real description', () => {
    expect(displayTitle('AMENDMENT TO MANAGEMENT AGREEMENT', '')).toBe('Amendment to Management Agreement');
    expect(displayTitle('EX-10.1 Credit Agreement', '')).toBe('EX-10.1 Credit Agreement');
  });
  it('caps a runaway first line', () => {
    expect(displayTitle('EX-10.1', 'word '.repeat(60)).length).toBeLessThanOrEqual(90);
  });
});

describe('textContent', () => {
  it('returns the visible text of a markdown slice, ignoring hr/comments/images', () => {
    expect(textContent('<hr>\n<!-- a.jpg --><hr>\n')).toBe('');
    expect(textContent('---\n\n![](a.jpg)\n\n***')).toBe('');
    expect(textContent('# Title\n\nBody **bold**.')).toBe('Title Body bold.');
  });
});

describe('resolveEdgarImages', () => {
  const url = 'https://www.sec.gov/Archives/edgar/data/1388658/000138865826000094/0001388658-26-000094.txt';
  it('rewrites relative image refs to the filing folder', () => {
    expect(resolveEdgarImages('![](irhythm-001.jpg "slide1")', url)).toBe(
      '![](https://www.sec.gov/Archives/edgar/data/1388658/000138865826000094/irhythm-001.jpg "slide1")',
    );
  });
  it('leaves absolute refs and non-image markdown alone', () => {
    expect(resolveEdgarImages('![x](https://hf/a.png) [link](foo.htm)', url)).toBe(
      '![x](https://hf/a.png) [link](foo.htm)',
    );
  });
  it('is a no-op without a filing url', () => {
    expect(resolveEdgarImages('![](a.jpg)', '')).toBe('![](a.jpg)');
  });
});

describe('ftsQuery', () => {
  it('quotes every token so FTS5 operators in user input are literal', () => {
    expect(ftsQuery('credit agreement')).toBe('"credit" "agreement"');
    expect(ftsQuery('  AND  "x" OR* ')).toBe('"AND" "x" "OR"');
    expect(ftsQuery('')).toBe('');
  });
  it('keeps a quoted phrase together', () => {
    expect(ftsQuery('"change of control" severance')).toBe('"change of control" "severance"');
  });
});

describe('filing text cannot inject marks; stray quotes split tokens (#86)', () => {
  it('escapes a literal <mark> that came from the filing, not from FTS5', () => {
    // FTS5 wraps hits in <mark>…</mark>; the filing itself said "<mark>" too.
    expect(snippetHtml(`a literal <mark> from the filing and ${SNIPPET_OPEN}hit${SNIPPET_CLOSE}`)).toBe(
      'a literal &lt;mark&gt; from the filing and <mark class="sl-mark">hit</mark>',
    );
  });
  it('treats a stray quote as a token boundary instead of fusing words', () => {
    expect(ftsQuery('foo"bar baz"')).toBe('"foo" "bar" "baz"');
    expect(ftsQuery('O"Neil')).toBe('"O" "Neil"');
    expect(ftsQuery('"credit agreement" lender')).toBe('"credit agreement" "lender"');
  });
});

describe('final review (#88)', () => {
  it('delimits hits with noncharacters, so private-use glyphs from filings stay text', () => {
    expect([SNIPPET_OPEN, SNIPPET_CLOSE]).toEqual(['\uFDD0', '\uFDD1']);
    expect(snippetHtml(`filing \uE000logo\uE001 rest ${SNIPPET_OPEN}hit${SNIPPET_CLOSE}`)).toBe(
      'filing \uE000logo\uE001 rest <mark class="sl-mark">hit</mark>',
    );
  });
  it('stays linear on long whitespace and dash runs', () => {
    const t0 = performance.now();
    snippetHtml(`…${' '.repeat(200_000)}x`);
    snippetHtml(`a ${'- '.repeat(50_000)}b`);
    snippetHtml(`a ${'-'.repeat(100_000)}:${' '.repeat(100_000)}b`);
    expect(performance.now() - t0).toBeLessThan(250);
  });
  it('keeps a mid-sentence ">" and strips blockquote markers at line starts', () => {
    expect(snippetHtml(fts('revenues > $10M and <mark>hit</mark>'))).toBe(
      'revenues &gt; $10M and <mark class="sl-mark">hit</mark>',
    );
    expect(snippetHtml(fts('line one\n> quoted <mark>x</mark>'))).toBe(
      'line one quoted <mark class="sl-mark">x</mark>',
    );
  });
  it('opens a phrase only at a word boundary', () => {
    expect(ftsQuery('O"Neil said "credit agreement"')).toBe('"O" "Neil" "said" "credit agreement"');
    expect(ftsQuery('("credit agreement")')).toBe('"credit agreement"');
  });
});

describe('review of the #84/#85 fixes (#87)', () => {
  it('never deletes prose before a stray --> at the start of a snippet', () => {
    expect(snippetHtml(fts('…units priced at $10 --> per the <mark>agreement</mark>'))).toBe(
      '…units priced at $10 per the <mark class="sl-mark">agreement</mark>',
    );
  });
  it('sees a comment whose marker hides a zero-width character', () => {
    expect(snippetHtml(fts('<!\uFEFF-- Document created using Wdesk --> Exhibit <mark>Agreement</mark>'))).toBe(
      'Exhibit <mark class="sl-mark">Agreement</mark>',
    );
  });
  it('drops Word downlevel conditional fragments', () => {
    expect(
      snippetHtml(
        fts('IN WITNESS WHEREOF <![endif]--> the <mark>parties</mark> <![if !supportLists]>1.<![endif]> agreed'),
      ),
    ).toBe('IN WITNESS WHEREOF the <mark class="sl-mark">parties</mark> 1. agreed');
  });
  it('returns nothing for an all-comment snippet and drops emptied hits', () => {
    expect(snippetHtml('<!-- Workiva page footer…')).toBe('');
    expect(snippetHtml(fts('<mark>\u200B</mark> tail'))).toBe('tail');
  });
  it('keeps escaped marker characters and a literal trailing backslash', () => {
    expect(textContent('aspi\\_ex101.htm SUBSCRIPTION')).toBe('aspi_ex101.htm SUBSCRIPTION');
    expect(textContent('3\\* shares')).toBe('3* shares');
    expect(textContent('path ends C:\\')).toBe('path ends C:\\');
    expect(textContent('AMENDMENT TO\\\nSTOCK PURCHASE')).toBe('AMENDMENT TO STOCK PURCHASE');
  });
});

describe('snippetHtml drops HTML comments and zero-width characters (#85)', () => {
  it('removes whole comments, including ones that contain a hit', () => {
    expect(
      snippetHtml(
        fts(
          '<!-- Document created using Wdesk --> <!-- Copyright 2026 Workiva --> Exhibit 10.1 Credit <mark>Agreement</mark>',
        ),
      ),
    ).toBe('Exhibit 10.1 Credit <mark class="sl-mark">Agreement</mark>');
    expect(snippetHtml(fts('<!-- <mark>Document</mark> created --> The <mark>Document</mark>'))).toBe(
      'The <mark class="sl-mark">Document</mark>',
    );
  });
  it('removes a comment cut in half by the snippet window', () => {
    expect(snippetHtml(fts('…11/6/2026 09:56:08 PM (UTC)--> SECOND AMENDMENT to <mark>agreement</mark>'))).toBe(
      '…11/6/2026 09:56:08 PM (UTC) SECOND AMENDMENT to <mark class="sl-mark">agreement</mark>',
    );
    expect(snippetHtml(fts('the <mark>agreement</mark> ends <!-- Workiva page footer…'))).toBe(
      'the <mark class="sl-mark">agreement</mark> ends…',
    );
  });
  it('keeps marks balanced when a cut comment swallows a closing tag', () => {
    expect(snippetHtml(fts('<mark>Docu <!-- x</mark> y'))).toBe('<mark class="sl-mark">Docu</mark>…');
  });
  it('drops zero-width characters', () => {
    expect(snippetHtml(fts('​SECOND‍ AMEND﻿MENT'))).toBe('SECOND AMENDMENT');
  });
  it('leaves a stray "-->" in prose alone unless the snippet starts mid-comment', () => {
    expect(snippetHtml(fts('a <!-- b --> c --> d'))).toBe('a c --&gt; d');
  });
});

describe('snippetHtml', () => {
  it('keeps the FTS <mark> tags while stripping markdown markers, including ">"', () => {
    expect(snippetHtml(fts('> **PLACEMENT** <mark>AGREEMENT</mark> | 5'))).toBe(
      'PLACEMENT <mark class="sl-mark">AGREEMENT</mark> 5',
    );
  });
  it('escapes any other markup in the source text', () => {
    expect(snippetHtml(fts('a <b>x</b> & <mark>y</mark>'))).toBe(
      'a &lt;b&gt;x&lt;/b&gt; &amp; <mark class="sl-mark">y</mark>',
    );
  });
  it('drops markdown table rules and horizontal rules', () => {
    expect(snippetHtml(fts('herein. - ----- | :---: | ------- 4.10. Under <mark>agreement</mark>'))).toBe(
      'herein. 4.10. Under <mark class="sl-mark">agreement</mark>',
    );
  });
  it('is empty for an empty snippet', () => {
    expect(snippetHtml(fts('  '))).toBe('');
  });
});
