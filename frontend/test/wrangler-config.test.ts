import { it, expect } from 'vitest';
import raw from '../wrangler.jsonc?raw';

// Strip // and /* */ comments outside of strings, then trailing commas, so the
// JSONC parses as JSON without pulling in a parser dependency.
function parseJsonc(src: string): any {
  let out = '';
  let i = 0;
  let inString = false;
  while (i < src.length) {
    const c = src[i];
    const next = src[i + 1];
    if (inString) {
      out += c;
      if (c === '\\') { out += next; i += 2; continue; }
      if (c === '"') inString = false;
      i += 1;
    } else if (c === '"') {
      inString = true; out += c; i += 1;
    } else if (c === '/' && next === '/') {
      while (i < src.length && src[i] !== '\n') i += 1;
    } else if (c === '/' && next === '*') {
      i = src.indexOf('*/', i + 2) + 2;
    } else {
      out += c; i += 1;
    }
  }
  return JSON.parse(out.replace(/,(\s*[}\]])/g, '$1'));
}

const cfg = parseJsonc(raw);
const HOST = 'live-contracts.arthur.law';

it('production claims its hostname twice: custom domain plus a host-specific zone route', () => {
  // The zone route is what keeps a wildcard route elsewhere on the zone from
  // shadowing the custom domain (the 2026-08-07 → 2026-10-02 outage).
  expect(cfg.routes).toEqual([
    { pattern: HOST, custom_domain: true },
    { pattern: `${HOST}/*`, zone_name: 'arthur.law' },
  ]);
});

it('staging never inherits the production hostname', () => {
  // `routes` is an inheritable key in wrangler: an environment without its own
  // `routes` deploys with the top-level ones, and a staging deploy would take
  // live-contracts.arthur.law away from production.
  const staging = cfg.env.staging;
  expect(staging.routes).toEqual([]);
  expect(staging.workers_dev).toBe(true);
  expect(JSON.stringify(staging)).not.toContain(HOST);
});

it('staging is bound to its own database and KV, never production\'s', () => {
  const staging = cfg.env.staging;
  expect(staging.name).toBe('sec-ex10-frontend-staging');
  expect(staging.d1_databases[0].database_name).toBe('sec-ex10-staging');
  expect(staging.d1_databases[0].database_id).not.toBe(cfg.d1_databases[0].database_id);
  expect(staging.kv_namespaces[0].id).not.toBe(cfg.kv_namespaces[0].id);
});
