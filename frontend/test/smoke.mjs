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

await check('homepage live feed', '/', '/ Live contracts', '/ Latest agreements', 'class="sl-feed"');
await check(
  'navbar wordmark',
  '/',
  'href="https://arthur.law/"',
  'ARTHUR<i>.</i>SOUZA<i>.</i>RODRIGUES',
  'class="mbar"',
);
await check('archive pagination', '/agreements/1', 'agreements</h1>', 'class="pagination"', 'class="sl-filters-col"');
await check('detail markdown shell', '/agreement/103', 'class="prose"', 'View original filing');
await check('search page form', '/search', 'class="sl-search-row', 'role="search"');
await check('search results', '/search?q=agreement', 'results for', 'class="sl-doc"');
{
  // A page past the end redirects to the last page instead of "N results" over "No matches" (#84).
  const first = await (await fetch(`${BASE}/search?q=agreement`)).text();
  const last = first.match(/class="current">1 \/ (\d+)</)?.[1];
  const res = await fetch(`${BASE}/search?q=agreement&page=99999`, { redirect: 'manual' });
  const loc = res.headers.get('location') ?? '';
  if (last && res.status === 302 && loc.endsWith(`/search?q=agreement&page=${last}`)) {
    console.log(`✓ search page past the end redirects (${loc})`);
  } else {
    failures += 1;
    console.error(`✗ search page past the end: status=${res.status} location=${loc}`);
  }
}

if (failures) {
  console.error(`\n${failures} smoke check(s) failed`);
  process.exit(1);
}
console.log('\nAll smoke checks passed');
