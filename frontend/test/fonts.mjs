/**
 * Font-loading guard (no live server needed).
 *
 * Canonical design-system primaries, both SELF-HOSTED (latin-subset, variable
 * woff2 in /public/fonts) — no external font request:
 *   - Libre Franklin — body + display  (--font-body)
 *   - Roboto Mono     — labels, wordmark, code (--font-mono)
 * Departure Mono is retained only as the documented mono fallback.
 *
 * This test fails if the build regresses to a Google-Fonts request, if a
 * primary stops being self-hosted, or if the token stacks lose their canonical
 * first family.
 *
 * Usage: node test/fonts.mjs
 */
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const base = readFileSync(join(here, '../src/layouts/Base.astro'), 'utf8');
const css = readFileSync(join(here, '../src/styles/global.css'), 'utf8');
const fontsDir = join(here, '../public/fonts');

let failures = 0;
function assert(cond, msg) {
  if (cond) {
    console.log(`✓ ${msg}`);
  } else {
    failures += 1;
    console.error(`✗ ${msg}`);
  }
}

// Files are non-empty (guard against accidental truncation).
assert(base.length > 0, 'Base.astro is non-empty');
assert(css.length > 0, 'global.css is non-empty');

// ---- No external font dependency ----
// Fonts are self-hosted; there must be no Google Fonts request anywhere.
assert(!/fonts\.googleapis\.com/i.test(base), 'Base.astro makes no Google Fonts request');
assert(!/fonts\.gstatic\.com/i.test(base), 'Base.astro has no fonts.gstatic.com preconnect');
assert(!/@import[^;]*fonts\.googleapis\.com/i.test(css), 'global.css has no Google Fonts @import');
// Manrope is retired (it was an alternate, never a canonical primary here).
assert(!/family=Manrope/i.test(base) && !/['"]Manrope['"]/i.test(css), 'Manrope is no longer loaded or referenced as a primary');
// No JetBrains Mono regression.
assert(!/JetBrains.{0,3}Mono/i.test(base), 'Base.astro does not request JetBrains Mono');
assert(!/JetBrains.{0,3}Mono/i.test(css), 'global.css does not reference JetBrains Mono');

// ---- The two canonical primaries are self-hosted woff2 ----
assert(existsSync(join(fontsDir, 'libre-franklin.woff2')), 'libre-franklin.woff2 is present in public/fonts');
assert(existsSync(join(fontsDir, 'roboto-mono.woff2')), 'roboto-mono.woff2 is present in public/fonts');

// Preload hints in Base.astro for both primaries (crossorigin required for fonts).
assert(/rel="preload"[^>]*libre-franklin\.woff2[^>]*crossorigin/i.test(base) ||
       /libre-franklin\.woff2[^>]*rel="preload"[^>]*crossorigin/i.test(base),
       'Base.astro preloads libre-franklin.woff2 (crossorigin)');
assert(/rel="preload"[^>]*roboto-mono\.woff2[^>]*crossorigin/i.test(base) ||
       /roboto-mono\.woff2[^>]*rel="preload"[^>]*crossorigin/i.test(base),
       'Base.astro preloads roboto-mono.woff2 (crossorigin)');

// ---- @font-face quality for each primary ----
function fontFaceFor(family) {
  const re = new RegExp(`@font-face\\s*\\{[^}]*${family}[^}]*\\}`, 'i');
  return css.match(re)?.[0] ?? '';
}
const lf = fontFaceFor('Libre Franklin');
const rm = fontFaceFor('Roboto Mono');
assert(/font-display\s*:\s*swap/i.test(lf), 'Libre Franklin @font-face sets font-display: swap');
assert(/format\(['"]?woff2['"]?\)/i.test(lf), 'Libre Franklin @font-face uses woff2');
assert(/font-display\s*:\s*swap/i.test(rm), 'Roboto Mono @font-face sets font-display: swap');
assert(/format\(['"]?woff2['"]?\)/i.test(rm), 'Roboto Mono @font-face uses woff2');

// ---- Token stacks reference the canonical first families ----
const bodyLine = css.match(/--font-body\s*:[^;]+/)?.[0] ?? '';
const monoLine = css.match(/--font-mono\s*:[^;]+/)?.[0] ?? '';
const firstBody = bodyLine.replace(/--font-body\s*:\s*/, '').split(',')[0].trim().replace(/['"]/g, '');
const firstMono = monoLine.replace(/--font-mono\s*:\s*/, '').split(',')[0].trim().replace(/['"]/g, '');
assert(firstBody === 'Libre Franklin', `--font-body first family is 'Libre Franklin' (found '${firstBody}')`);
assert(firstMono === 'Roboto Mono', `--font-mono first family is 'Roboto Mono' (found '${firstMono}')`);

if (failures) {
  console.error(`\n${failures} font check(s) failed`);
  process.exit(1);
}

console.log('\nAll font checks passed');
