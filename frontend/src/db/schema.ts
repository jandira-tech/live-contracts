import { sqliteTable, text, index, uniqueIndex } from 'drizzle-orm/sqlite-core';

// Mirrors the SQLite ex10_exhibits table. filed_at is stored explicitly
// (extracted from filing_metadata.filed_at at ingest) so it can be indexed.
export const exhibits = sqliteTable(
  'exhibits',
  {
    // UUIDv7 string, assigned by /api/ingest. The producer's local SQLite id is
    // volatile (resets on reseed) and collided with existing rows when used as the
    // D1 PK; a UUIDv7 is globally unique and time-ordered (sortable).
    id: text('id').primaryKey(),
    accession: text('accession').notNull(),
    cik: text('cik'),
    formType: text('form_type'),
    docType: text('doc_type'),
    filename: text('filename').notNull(),
    description: text('description'),
    sequence: text('sequence'),
    filingUrl: text('filing_url'),
    foundAt: text('found_at'),
    filedAt: text('filed_at'),
    markdownStatus: text('markdown_status'),
    filingMetadata: text('filing_metadata'),
    imageUrls: text('image_urls'),
    markdown: text('markdown'),
  },
  (t) => ({
    // Matches the source SQLite UNIQUE(accession, doc_type, filename) exactly —
    // do NOT drop doc_type or distinct exhibits get coalesced.
    uniqAccDocFile: uniqueIndex('uniq_acc_doc_file').on(t.accession, t.docType, t.filename),
    idxFiledAt: index('idx_ex_filed_at').on(t.filedAt),
    idxForm: index('idx_ex_form_type').on(t.formType),
    idxCik: index('idx_ex_cik').on(t.cik),
  }),
);

export type ExhibitRow = typeof exhibits.$inferSelect;
export type ExhibitInsert = typeof exhibits.$inferInsert;
