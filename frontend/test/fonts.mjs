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

if (failures) {
  console.error(`\n${failures} font check(s) failed`);
  process.exit(1);
}
console.log('\nAll font checks passed');
