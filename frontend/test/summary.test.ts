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
      id: '1', accession: 'acc-1', cik: '111', formType: '8-K', docType: 'EX-10.1',
      filename: 'test.htm', description: 'A test agreement', filingUrl: 'https://sec/test',
      foundAt: '2026-01-01T00:00:00', filedAt: '20260101000000', markdownStatus: 'done',
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
      id: '2', accession: 'acc-2', cik: '222', formType: '10-Q', docType: 'EX-10.2',
      filename: 'b.htm', description: '', filingUrl: 'https://sec/b',
      foundAt: '2026-01-02T00:00:00', filedAt: '', markdownStatus: 'pending',
      filingMetadata: null, imageUrls: null, markdown: null,
    };

    const summary = rowToSummary(row);

    expect(summary.has_markdown).toBe(false);
    expect(summary.excerpt).toBe('');
    expect(summary.excerpt_md).toBe('');
  });
});