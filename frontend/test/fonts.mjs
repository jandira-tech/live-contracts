/**
 * Font-loading guard (no live server needed).
 *
 * The design system uses exactly TWO families:
 *   - Manrope         — body, loaded once from Google Fonts in Base.astro
 *   - Departure Mono  — mono, self-hosted via @font-face in global.css
 *
 * JetBrains Mono was a redundant SECOND mono font (Google Fonts fallback). It
 * is removed so we don't ship two mono webfonts. This test fails if it returns,
 * or if the single Google Fonts stylesheet is ever duplicated.
 *
 * Usage: node test/fonts.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const base = readFileSync(join(here, '../src/layouts/Base.astro'), 'utf8');
const css = readFileSync(join(here, '../src/styles/global.css'), 'utf8');

let failures = 0;
function assert(cond, msg) {
  if (cond) {
    console.log(`✓ ${msg}`);
  } else {
    failures += 1;
    console.error(`✗ ${msg}`);
  }
}

// No JetBrains Mono anywhere — neither the Google request nor the --font-mono stack.
assert(!/JetBrains\+?Mono/i.test(base), 'Base.astro does not request JetBrains Mono');
assert(!/JetBrains Mono/i.test(css), 'global.css --font-mono drops JetBrains Mono');

// Exactly one Google Fonts stylesheet link (no double import).
const sheetLinks = (base.match(/fonts\.googleapis\.com\/css2/g) || []).length;
assert(sheetLinks === 1, `exactly one Google Fonts stylesheet link (found ${sheetLinks})`);

// The two families we keep are still present.
assert(/family=Manrope/i.test(base), 'Manrope is still loaded from Google Fonts');
assert(/@font-face[\s\S]*Departure Mono/i.test(css), 'Departure Mono @font-face is self-hosted');

// ---- Additional regression / boundary checks ----

// Files are non-empty (guard against accidental truncation).
assert(base.length > 0, 'Base.astro is non-empty');
assert(css.length > 0, 'global.css is non-empty');

// Preconnect hints — both origins must be present so the browser can warm up the
// connection before the stylesheet link is parsed.
assert(/rel="preconnect"[\s\S]*?href="https:\/\/fonts\.googleapis\.com"/i.test(base) ||
       /href="https:\/\/fonts\.googleapis\.com"[\s\S]*?rel="preconnect"/i.test(base),
       'Base.astro has <link rel="preconnect"> to fonts.googleapis.com');
assert(/rel="preconnect"[\s\S]*?href="https:\/\/fonts\.gstatic\.com"/i.test(base) ||
       /href="https:\/\/fonts\.gstatic\.com"[\s\S]*?rel="preconnect"/i.test(base),
       'Base.astro has <link rel="preconnect"> to fonts.gstatic.com');

// The gstatic preconnect must carry crossorigin so the browser can share the
// connection for CORS font requests.
assert(/fonts\.gstatic\.com[^>]*crossorigin/i.test(base),
       'fonts.gstatic.com preconnect carries crossorigin attribute');

// Google Fonts URL must include display=swap to avoid invisible text during load.
assert(/fonts\.googleapis\.com\/css2[^"]*display=swap/i.test(base),
       'Google Fonts URL includes display=swap');

// Manrope weight variants — the design tokens need 300, 400, 500, 600, 700.
const manropeUrl = base.match(/fonts\.googleapis\.com\/css2[^"']*/)?.[0] ?? '';
assert(/300/.test(manropeUrl), 'Manrope weight 300 requested from Google Fonts');
assert(/400/.test(manropeUrl), 'Manrope weight 400 requested from Google Fonts');
assert(/500/.test(manropeUrl), 'Manrope weight 500 requested from Google Fonts');
assert(/600/.test(manropeUrl), 'Manrope weight 600 requested from Google Fonts');
assert(/700/.test(manropeUrl), 'Manrope weight 700 requested from Google Fonts');

// No @import of Google Fonts inside the CSS file — that would create a
// render-blocking waterfall. The preconnect + <link> in Base.astro is the
// canonical loading path.
assert(!/@import[^;]*fonts\.googleapis\.com/i.test(css),
       'global.css has no @import for Google Fonts (avoids render-blocking waterfall)');

// @font-face quality checks for Departure Mono.
const fontFaceBlock = css.match(/@font-face\s*\{[^}]+\}/)?.[0] ?? '';
assert(/font-display\s*:\s*swap/i.test(fontFaceBlock),
       'Departure Mono @font-face sets font-display: swap');
assert(/\.woff2[^;]*format\(['"]?woff2['"]?\)/i.test(fontFaceBlock) ||
       /format\(['"]?woff2['"]?\)/i.test(fontFaceBlock),
       'Departure Mono @font-face includes woff2 format hint');
assert(/font-weight\s*:\s*400/i.test(fontFaceBlock),
       'Departure Mono @font-face declares font-weight: 400');

// CSS token checks — the design-token variables must reference the correct families.
assert(/--font-body\s*:[^;]*['"]?Manrope['"]?/i.test(css),
       'global.css --font-body token references Manrope');
assert(/--font-mono\s*:[^;]*['"]?Departure Mono['"]?/i.test(css),
       'global.css --font-mono token references Departure Mono');

// Departure Mono must be the FIRST entry in --font-mono (primary font, not a fallback).
const fontMonoLine = css.match(/--font-mono\s*:[^;]+/)?.[0] ?? '';
const firstMonoFamily = fontMonoLine.replace(/--font-mono\s*:\s*/, '').trim().split(',')[0].trim().replace(/['"]/g, '');
assert(firstMonoFamily === 'Departure Mono',
       `--font-mono first family is 'Departure Mono' (found '${firstMonoFamily}')`);

// JetBrains must not appear anywhere in the --font-mono stack (regression: removal must be complete).
assert(!/JetBrains/i.test(fontMonoLine),
       '--font-mono stack contains no JetBrains reference');

// Departure Mono must NOT be loaded from Google Fonts — it is self-hosted only.
assert(!manropeUrl.includes('Departure') && !/family=Departure/i.test(base),
       'Departure Mono is not fetched from Google Fonts (self-hosted only)');

if (failures) {
  console.error(`\n${failures} font check(s) failed`);
  process.exit(1);

console.log('\nAll font checks passed');
