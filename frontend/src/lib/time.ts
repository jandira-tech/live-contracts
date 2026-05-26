/**
 * Format a SQLite UTC timestamp ("YYYY-MM-DD HH:MM:SS", stored in UTC) as
 * New York time (America/New_York, ET — handles EST/EDT automatically).
 */
/** Format an SEC YYYYMMDD date (e.g. period of report) as "May 19, 2026". */
export function formatYmd(ymd: string | null | undefined): string {
  if (!ymd || !/^\d{8}$/.test(ymd)) return ymd ?? '';
  const d = new Date(`${ymd.slice(0, 4)}-${ymd.slice(4, 6)}-${ymd.slice(6, 8)}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) return ymd;
  return new Intl.DateTimeFormat('en-US', {
    timeZone: 'UTC',
    year: 'numeric',
    month: 'short',
    day: '2-digit',
  }).format(d);
}

/**
 * Format an SEC acceptance datetime ("YYYYMMDDHHMMSS", already Eastern Time) as
 * "May 26, 2026, 16:16 ET". This is the actual filing acceptance time.
 */
export function formatFiledAt(s: string | null | undefined): string {
  if (!s || !/^\d{14}$/.test(s)) return '';
  const d = new Date(
    `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}T${s.slice(8, 10)}:${s.slice(10, 12)}:${s.slice(12, 14)}Z`,
  );
  if (Number.isNaN(d.getTime())) return '';
  const fmt = new Intl.DateTimeFormat('en-US', {
    timeZone: 'UTC', // value is already ET wall-clock; don't shift it
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(d);
  return `${fmt} ET`;
}

export function formatNY(utc: string | null | undefined): string {
  if (!utc) return '';
  const norm = utc.includes('T') ? utc : utc.replace(' ', 'T');
  const d = new Date(norm.endsWith('Z') ? norm : `${norm}Z`);
  if (Number.isNaN(d.getTime())) return utc;
  return new Intl.DateTimeFormat('en-US', {
    timeZone: 'America/New_York',
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
    timeZoneName: 'short',
  }).format(d);
}
