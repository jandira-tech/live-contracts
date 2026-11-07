#!/usr/bin/env node
/**
 * Backfill the FTS5 index (migration 0003) for rows that predate its triggers.
 * Runs `wrangler d1 execute` with an argument array (never a shell string), one
 * chunk at a time. Logic and safety checks live in fts-backfill-core.ts.
 *
 *   pnpm db:fts-backfill -- --remote --to <max rowid>          # production, producer stopped
 *   pnpm db:fts-backfill -- --remote --to <M> --from <last hi>  # resume after an abort
 *   pnpm db:fts-backfill -- --local [--persist-to <dir>]        # local miniflare D1
 *
 * Try `INSERT INTO exhibits_fts(exhibits_fts) VALUES('rebuild')` first: it is one
 * atomic statement and rolls back cleanly if it exceeds D1's time budget.
 */
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import {
  EXIT,
  parseArgs,
  parseWranglerJson,
  runBackfill,
  type Deps,
  type ExecResult,
  type Opts,
} from './fts-backfill-core.ts';

const run = promisify(execFile);
const PROBE_TIMEOUT_MS = 10_000;

function wranglerExec(opts: Opts) {
  return async (sql: string): Promise<ExecResult> => {
    const args = ['exec', 'wrangler', 'd1', 'execute', opts.db, `--${opts.mode}`, '--json', '--command', sql];
    if (opts.persistTo) args.push('--persist-to', opts.persistTo);
    let stdout: string;
    try {
      ({ stdout } = await run('pnpm', args, { maxBuffer: 64 * 1024 * 1024 }));
    } catch (e) {
      const err = e as { stdout?: string; stderr?: string; message: string };
      throw new Error(lastJsonError(err.stdout) ?? err.stderr?.trim() ?? err.message);
    }
    return parseWranglerJson(stdout);
  };
}

function lastJsonError(stdout?: string): string | undefined {
  const i = stdout?.search(/^\{/m) ?? -1;
  if (!stdout || i < 0) return undefined;
  try {
    const e = JSON.parse(stdout.slice(i)).error;
    return [e?.text, ...(e?.notes ?? []).map((n: { text: string }) => n.text)].filter(Boolean).join(': ');
  } catch {
    return undefined;
  }
}

async function main(): Promise<number> {
  let opts: Opts;
  try {
    opts = parseArgs(process.argv.slice(2).filter((a: string) => a !== '--'));
  } catch (e) {
    console.error((e as Error).message);
    return EXIT.USAGE;
  }
  const deps: Deps = {
    exec: wranglerExec(opts),
    probe: async (url) =>
      (await fetch(url, { headers: { 'user-agent': 'fts-backfill' }, signal: AbortSignal.timeout(PROBE_TIMEOUT_MS) }))
        .status,
    log: (line) => console.log(line),
    sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  };
  console.log(`backfill ${opts.db} (${opts.mode}) from=${opts.from} to=${opts.to ?? 'max(rowid)'} chunk=${opts.chunk}`);
  return (await runBackfill(deps, opts)).code;
}

process.exitCode = await main().catch((e: unknown) => {
  console.error(`EXEC_FAILED: ${(e as Error).message}`);
  return EXIT.EXEC_FAILED;
});
