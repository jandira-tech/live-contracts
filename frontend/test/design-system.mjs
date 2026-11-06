/**
 * Design-system conformance guard (no live server, no deps).
 *
 * Asserts the frontend follows the Arthur Souza Rodrigues design system and the
 * shared navbar convention used by prompting-for-attorneys:
 *   - wordmark ARTHUR.SOUZA.RODRIGUES links to https://arthur.law/
 *   - "/ Live contracts" sub-label links to the site root
 *   - DS tokens (--bg --ink --muted --faint --mark --page-max --px) are defined
 *   - no emoji in templates (the system is icon-free)
 *   - no mono label below the 11px floor
 *   - radius stays 0, mobile bottom bar exists for the three primary routes
 *
 * Usage: node test/design-system.mjs
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, '../src');
const base = readFileSync(join(src, 'layouts/Base.astro'), 'utf8');
const css = readFileSync(join(src, 'styles/global.css'), 'utf8');

function walk(dir, out = []) {
  for (const n of readdirSync(dir)) {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (p.endsWith('.astro')) out.push(p);
  }
  return out;
}
const templates = walk(src).map((p) => [p.replace(src, 'src'), readFileSync(p, 'utf8')]);

let failures = 0;
function assert(cond, msg) {
  if (cond) console.log(`✓ ${msg}`);
  else { failures += 1; console.error(`✗ ${msg}`); }
}

// ---- Navbar convention (shared with prompting-for-attorneys) ----
const wm = base.match(/<a[^>]*class="[^"]*\bwm\b[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/)
  ?? base.match(/<a[^>]*href="([^"]+)"[^>]*class="[^"]*\bwm\b[^"]*"[^>]*>([\s\S]*?)<\/a>/);
assert(wm !== null, 'Base.astro has a .wm wordmark anchor');
assert(wm?.[1] === 'https://arthur.law/', `wordmark links to https://arthur.law/ (found ${wm?.[1]})`);
const wmText = (wm?.[2] ?? '').replace(/<[^>]+>/g, '').replace(/\s+/g, '');
assert(wmText === 'ARTHUR.SOUZA.RODRIGUES', `wordmark text is ARTHUR.SOUZA.RODRIGUES (found ${wmText})`);
// Astro's class:list={['sub', ...]} or a plain class="sub ..." both count.
const sub = base.match(/<a[^>]*(?:class="[^"]*\bsub\b[^"]*"|class:list=\{\['sub'[^}]*\})[^>]*>([\s\S]*?)<\/a>/);
assert(sub !== null && sub[1].trim() === '/ Live contracts', 'sub-label reads "/ Live contracts"');
assert(sub !== null && /href="\/"/.test(sub[0]), 'sub-label links to the site root');
assert(!/LIVECONTRACTS/.test(base), 'old LIVECONTRACTS wordmark is gone');
assert(!/ABOUT ME/.test(base), 'redundant ABOUT ME link is gone (wordmark carries it)');
assert(/class="mbar"/.test(base), 'mobile bottom bar (.mbar) is present');
for (const r of ['href="/"', 'href="/agreements/1"', 'href="/search"']) {
  const m = base.match(/<nav class="mbar"[\s\S]*?<\/nav>/)?.[0] ?? '';
  assert(m.includes(r), `mobile bar links ${r}`);
}

assert(/class="nav-contact"[^>]*href="https:\/\/arthur\.law\/contact\/"/.test(base), 'header has a Contact link to arthur.law/contact/ (stays visible on phones)');
assert(/<noscript>\s*\{?.*http-equiv="refresh"/.test(base) || /<noscript><meta http-equiv="refresh"/.test(base), 'meta refresh only inside <noscript> (JS polls in place)');
const docket = readFileSync(join(src, 'components/DocketRow.astro'), 'utf8');
assert(/<article class="sl-doc" data-id=\{item\.id\}>/.test(docket), 'docket card carries data-id (live diffing)');
assert(/\.sl-doc-rail h3 a::after \{ content: ''; position: absolute; inset: 0;/.test(css), 'docket card is a stretched link (whole card clickable)');

// ---- Tokens ----
for (const t of ['--bg', '--ink', '--muted', '--faint', '--mark', '--mark-text', '--page-max', '--px', '--lift', '--ease']) {
  assert(new RegExp(`${t.replace(/-/g, '\\-')}\\s*:`).test(css), `global.css defines ${t}`);
}
assert(/--ink\s*:\s*#0A0A0A/i.test(css), '--ink is #0A0A0A');
assert(/--muted\s*:\s*#6B6B6B/i.test(css), '--muted is the DS #6B6B6B');
assert(/--mark\s*:\s*#FFD400/i.test(css), '--mark is #FFD400');
assert(/--page-max\s*:\s*1180px/.test(css), '--page-max is 1180px');
assert(/--radius\s*:\s*0(px)?\s*;/.test(css), '--radius is 0');
assert(!/border-radius\s*:(?!\s*(0(px)?|var\(--radius\)|50%)\s*[;}])/.test(css), 'no non-zero border-radius (50% allowed only for the status pulse)');
assert(!/box-shadow\s*:[^;]*blur|box-shadow\s*:\s*\d+px\s+\d+px\s+[1-9]\d*px/.test(css), 'no blur shadows (only hard offset blocks)');

// ---- Type floors ----
const small = [...css.matchAll(/font-size\s*:\s*([\d.]+)(px|rem)/g)]
  .map((m) => (m[2] === 'rem' ? parseFloat(m[1]) * 16 : parseFloat(m[1])))
  .filter((px) => px < 11);
assert(small.length === 0, `no font-size below 11px (found ${small.length})`);

// ---- Icon-free, emoji-free ----
const emoji = /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/u;
for (const [name, body] of templates) assert(!emoji.test(body), `${name} has no emoji`);

if (failures) { console.error(`\n${failures} design-system check(s) failed`); process.exit(1); }
console.log('\nAll design-system checks passed');
