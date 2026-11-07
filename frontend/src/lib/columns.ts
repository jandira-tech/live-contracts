/**
 * Column list, sort keys and page math shared by the list and search queries.
 */
import { asc, desc, sql } from 'drizzle-orm';
import { exhibits } from '../db/schema';

// substr keeps list payloads light (markdown bodies avg ~67KB).
export const summaryCols = {
  id: exhibits.id,
  accession: exhibits.accession,
  cik: exhibits.cik,
  formType: exhibits.formType,
  docType: exhibits.docType,
  filename: exhibits.filename,
  description: exhibits.description,
  sequence: exhibits.sequence,
  filingUrl: exhibits.filingUrl,
  foundAt: exhibits.foundAt,
  filedAt: exhibits.filedAt,
  markdownStatus: exhibits.markdownStatus,
  filingMetadata: exhibits.filingMetadata,
  imageUrls: exhibits.imageUrls,
  markdown: sql<string>`substr(${exhibits.markdown}, 1, 2000)`,
};
// Bare filed_at only. SQLite treats NULL and '' as smaller than any stamp, so
// DESC already places undated rows last and ASC places them first. Wrapping
// filed_at in `IS NULL OR = ''` (or adding found_at/id) makes D1 ignore
// idx_ex_filed_at, scan every markdown blob, and then fail the rest of the
// request with "Too many resources consumed within a 10 second window".
export const orderNewest = [desc(exhibits.filedAt)];
export const orderOldest = [asc(exhibits.filedAt)];

export function pages(total: number, size: number) {
  return total ? Math.max(1, Math.ceil(total / size)) : 0;
}
