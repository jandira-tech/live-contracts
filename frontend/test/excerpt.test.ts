import { describe, it, expect } from 'vitest';
import { cleanExcerpt } from '../src/lib/summary';

describe('cleanExcerpt', () => {
  it('strips a leading exhibit label and image refs, keeps body', () => {
    expect(cleanExcerpt('Exhibit 10.1 (logo.jpg) The parties agree as follows.'))
      .toBe('The parties agree as follows.');
  });
  it('preserves a mid-body exhibit reference', () => {
    expect(cleanExcerpt('The lease, subject to Exhibit 10.2, is binding.'))
      .toContain('subject to Exhibit 10.2');
  });
  it('truncates on a boundary with an ellipsis', () => {
    const s = cleanExcerpt('word '.repeat(200), 20);
    expect(s.endsWith('…')).toBe(true);
    expect(s.length).toBeLessThanOrEqual(21);
  });
  it('empty / nullish → empty string', () => {
    expect(cleanExcerpt('')).toBe('');
    expect(cleanExcerpt(null)).toBe('');
    expect(cleanExcerpt(undefined)).toBe('');
  });
});
