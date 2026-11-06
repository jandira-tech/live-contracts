#!/usr/bin/env bun
/**
 * One-off, resumable backfill of the FTS5 index (migration 0003) for rows that
 * existed before the triggers did. Runs through wrangler so it works against the
 * local miniflare D1 and the remote database alike, in chunks small enough for
 * D1's per-statement limits. Re-running is safe: it only indexes missing ids.
 *
 *   bun run db:fts-backfill            # local
 *   bun run db:fts-backfill -- --remote
 *   bun run db:fts-backfill -- --remote --env staging
 */
const args = process.argv.slice(2);
const remote = args.includes('--remote');
const envIdx = args.indexOf('--env');
const env = envIdx >= 0 ? args[envIdx + 1] : undefined;
const CHUNK = Number(process.env.FTS_CHUNK ?? 250);
const DB = env === 'staging' ? 'sec-ex10-staging' : 'sec-ex10';

const SQL = `INSERT INTO exhibits_fts(id, description, markdown)
  SELECT e.id, coalesce(e.description, ''), coalesce(e.markdown, '')
  FROM exhibits e WHERE NOT EXISTS (SELECT 1 FROM exhibits_fts f WHERE f.id = e.id)
  ORDER BY e.id LIMIT ${CHUNK}`;

async function run(): Promise<number> {
  const cmd = ['bunx', 'wrangler', 'd1', 'execute', DB, remote ? '--remote' : '--local', '--json', '--command', SQL];
  if (env) cmd.push('--env', env);
  const proc = Bun.spawn(cmd, { stdout: 'pipe', stderr: 'pipe' });
  const out = await new Response(proc.stdout).text();
  const err = await new Response(proc.stderr).text();
  if ((await proc.exited) !== 0) throw new Error(err || out);
  const json = JSON.parse(out.slice(out.indexOf('[')));
  return Number(json?.[0]?.meta?.changes ?? 0);
}

let total = 0;
for (;;) {
  const n = await run();
  total += n;
  console.log(`indexed ${n} (total ${total})`);
  if (n < CHUNK) break;
}
console.log(`done: ${total} rows added to exhibits_fts`);
