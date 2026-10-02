import type { APIRoute } from 'astro';
import { ex10Freshness } from '../lib/api';

// Machine-readable freshness: row count plus the newest filing and capture times.
// Read by the staleness alert (.github/workflows/staleness.yml), by smoke-prod.sh
// and by anything that prints a "live" claim next to a number. Edge-cached for five
// minutes so a monitor cannot turn into D1 load.
export const prerender = false;

export const GET: APIRoute = async () => {
  try {
    return new Response(JSON.stringify(await ex10Freshness()), {
      status: 200,
      headers: {
        'content-type': 'application/json; charset=utf-8',
        'cache-control': 'public, max-age=300',
        'access-control-allow-origin': '*',
      },
    });
  } catch {
    // A monitor must see an error as an error, never as a cached "fresh".
    return new Response(JSON.stringify({ error: 'stats unavailable' }), {
      status: 503,
      headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' },
    });
  }
};
