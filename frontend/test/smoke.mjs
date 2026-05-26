/**
 * Frontend route smoke test.
 *
 * Asserts the running dev/preview server renders each route type with real data:
 * live homepage, native-paginated archive, markdown detail, and the search page.
 *
 * Usage: BASE=http://localhost:4321 node test/smoke.mjs
 * Requires the Astro server (astro dev / preview) AND the internal API running.
 */
const BASE = process.env.BASE ?? 'http://localhost:4321';

let failures = 0;
async function check(name, path, ...needles) {
  const res = await fetch(`${BASE}${path}`);
  const body = await res.text();
  const okStatus = res.status === 200;
  const missing = needles.filter((n) => !body.includes(n));
  if (okStatus && missing.length === 0) {
    console.log(`✓ ${name} (${path})`);
  } else {
    failures += 1;
    console.error(`✗ ${name} (${path}) status=${res.status} missing=${JSON.stringify(missing)}`);
  }
}

await check('homepage live banner', '/', 'last 60 seconds', 'LIVE');
await check('archive pagination', '/agreements/1', 'All EX-10 agreements', 'class="pagination"');
await check('detail markdown shell', '/agreement/103', 'class="prose"', 'View original filing');
await check('search page', '/search', 'Search agreements', 'pagefind-ui.js');

if (failures) {
  console.error(`\n${failures} smoke check(s) failed`);
  process.exit(1);
}
console.log('\nAll smoke checks passed');
