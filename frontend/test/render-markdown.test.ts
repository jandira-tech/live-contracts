import { describe, it, expect } from 'vitest';
import { closeRunInStrong, renderMarkdown } from '../src/lib/render-markdown';

describe('closeRunInStrong', () => {
  it('closes a run-in heading whose space sits inside the markers', () => {
    const src =
      '**1. Engagement. **Subject to the terms and conditions of this Agreement, the Company hereby engages CEO';
    expect(closeRunInStrong(src)).toBe(
      '**1. Engagement.** Subject to the terms and conditions of this Agreement, the Company hereby engages CEO',
    );
    expect(renderMarkdown(src)).toBe(
      '<p><strong>1. Engagement.</strong> Subject to the terms and conditions of this Agreement, the Company hereby engages CEO</p>\n',
    );
  });

  it('closes an escaped list marker and a defined name', () => {
    expect(closeRunInStrong('**1\\. Engagement. **Subject')).toBe('**1\\. Engagement.** Subject');
    expect(closeRunInStrong('**Yongrong He **(the **“Executive”**).')).toBe('**Yongrong He** (the **“Executive”**).');
    expect(closeRunInStrong('This **SECURITIES PURCHASE AGREEMENT **(the “Agreement”)')).toBe(
      'This **SECURITIES PURCHASE AGREEMENT** (the “Agreement”)',
    );
  });

  it('does not re-pair adjacent bolds, tables, triple asterisks, italics, or code', () => {
    const same = [
      '| **Principal Amount: $2,000,000**                | **Issue Date: October 3, 2026** |',
      '**FOR VALUE RECEIVED**, **CLEAN ENERGY TECHNOLOGIES, INC.**, a Nevada corporation',
      '**Section 3**. ***Reference***. On and after',
      '**This Consulting Agreement**** **(the “Agreement”)',
      '1. *Engagement*. Upon the terms',
      'keep `**foo **` literal',
      '| **RE:** | **LETTER OF OFFER** |',
    ];
    for (const src of same) expect(closeRunInStrong(src)).toBe(src);
  });
});
