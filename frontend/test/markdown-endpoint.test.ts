import { describe, it, expect, beforeEach } from 'vitest';
import { seed } from './seed';
import { GET } from '../src/pages/api/markdown/[id]';

beforeEach(async () => { await seed(); });

describe('GET /api/markdown/[id]', () => {
  it('returns the raw markdown as text/markdown with permissive CORS', async () => {
    const res = await GET({ params: { id: '1' } } as any);
    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toContain('text/markdown');
    expect(res.headers.get('Access-Control-Allow-Origin')).toBe('*');
    expect(await res.text()).toContain('Alpha contract body about leasing.');
  });
  it('404s for a missing id', async () => {
    expect((await GET({ params: { id: '999' } } as any)).status).toBe(404);
  });
  it('404s for a non-numeric id', async () => {
    expect((await GET({ params: { id: 'abc' } } as any)).status).toBe(404);
  });
});
