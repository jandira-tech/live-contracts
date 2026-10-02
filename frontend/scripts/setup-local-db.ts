import Database from 'better-sqlite3';
import { readFileSync, readdirSync, existsSync, unlinkSync } from 'fs';
import { join } from 'path';

const DB_PATH = './.local-db.sqlite';

// Delete existing database if it exists to ensure idempotency
if (existsSync(DB_PATH)) {
  unlinkSync(DB_PATH);
  console.log(`Deleted existing database at ${DB_PATH}`);
}

// Create fresh local database
const db = new Database(DB_PATH);

// Load and run all migrations in order
const migrationsDir = './migrations';
const migrationFiles = readdirSync(migrationsDir)
  .filter(f => f.endsWith('.sql') && f !== 'meta')
  .sort();

console.log(`Running ${migrationFiles.length} migrations...`);

for (const file of migrationFiles) {
  const sql = readFileSync(join(migrationsDir, file), 'utf-8');
  const statements = sql.split('--> statement-breakpoint');
  
  for (const stmt of statements) {
    const s = stmt.trim();
    if (s) {
      db.exec(s.replace(/\n/g, ' '));
    }
  }
  console.log(`  ✓ ${file}`);
}

// Seed with dummy data
console.log('Seeding dummy data...');
const insertStmt = db.prepare(`
  INSERT INTO exhibits (
    id, accession, cik, form_type, doc_type, filename, description, sequence,
    filing_url, found_at, filed_at, markdown_status, filing_metadata, image_urls, markdown
  ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
`);

const dummyData = [
  {
    id: '1', accession: '0001193125-26-123456', cik: '0000320193', formType: '8-K', docType: 'EX-10.1',
    filename: 'ex10-1.htm', description: 'Material Commercial Agreement', sequence: '1',
    filingUrl: 'https://www.sec.gov/Archives/edgar/data/320193/0001193125-26-123456-index.htm',
    foundAt: '2026-07-01T12:00:00Z', filedAt: '20260701120000', markdownStatus: 'done',
    filingMetadata: JSON.stringify({ company_name: 'Apple Inc.', filed_at: '20260701120000', items: ['8.01'] }),
    imageUrls: '[]', markdown: '# Material Commercial Agreement\n\nThis agreement is entered into on July 1, 2026...'
  },
  {
    id: '2', accession: '0001193125-26-234567', cik: '0000789019', formType: '10-Q', docType: 'EX-10.2',
    filename: 'ex10-2.htm', description: 'License Agreement', sequence: '2',
    filingUrl: 'https://www.sec.gov/Archives/edgar/data/789019/0001193125-26-234567-index.htm',
    foundAt: '2026-07-01T14:30:00Z', filedAt: '20260701143000', markdownStatus: 'done',
    filingMetadata: JSON.stringify({ company_name: 'Microsoft Corporation', filed_at: '20260701143000', items: ['10.01'] }),
    imageUrls: '[]', markdown: '# License Agreement\n\nThis license agreement grants the licensee...'
  },
  {
    id: '3', accession: '0001193125-26-345678', cik: '0001067983', formType: '8-K', docType: 'EX-10.3',
    filename: 'ex10-3.htm', description: 'Employment Agreement', sequence: '1',
    filingUrl: 'https://www.sec.gov/Archives/edgar/data/1067983/0001193125-26-345678-index.htm',
    foundAt: '2026-06-30T09:15:00Z', filedAt: '20260630091500', markdownStatus: 'pending',
    filingMetadata: JSON.stringify({ company_name: 'Berkshire Hathaway Inc.', filed_at: '20260630091500', items: ['8.01'] }),
    imageUrls: '[]', markdown: ''
  },
  {
    id: '4', accession: '0001193125-26-456789', cik: '0001326801', formType: '10-K', docType: 'EX-10.4',
    filename: 'ex10-4.htm', description: 'Supply Agreement', sequence: '4',
    filingUrl: 'https://www.sec.gov/Archives/edgar/data/1326801/0001193125-26-456789-index.htm',
    foundAt: '2026-06-29T16:45:00Z', filedAt: '20260629164500', markdownStatus: 'done',
    filingMetadata: JSON.stringify({ company_name: 'Meta Platforms, Inc.', filed_at: '20260629164500', items: ['10.01'] }),
    imageUrls: '[]', markdown: '# Supply Agreement\n\nThis supply agreement is effective as of June 29, 2026...'
  },
  {
    id: '5', accession: '0001193125-26-567890', cik: '0001652044', formType: '8-K', docType: 'EX-10.5',
    filename: 'ex10-5.htm', description: 'Merger Agreement', sequence: '1',
    filingUrl: 'https://www.sec.gov/Archives/edgar/data/1652044/0001193125-26-567890-index.htm',
    foundAt: '2026-06-28T11:20:00Z', filedAt: '20260628112000', markdownStatus: 'done',
    filingMetadata: JSON.stringify({ company_name: 'Alphabet Inc.', filed_at: '20260628112000', items: ['8.01'] }),
    imageUrls: '[]', markdown: '# Merger Agreement\n\nThis merger agreement is made between the parties...'
  },
];

for (const row of dummyData) {
  insertStmt.run(
    row.id, row.accession, row.cik, row.formType, row.docType, row.filename,
    row.description, row.sequence, row.filingUrl, row.foundAt, row.filedAt,
    row.markdownStatus, row.filingMetadata, row.imageUrls, row.markdown
  );
}

console.log(`  ✓ Inserted ${dummyData.length} dummy records`);

// Verify
const count = db.prepare('SELECT COUNT(*) as n FROM exhibits').get() as { n: number };
console.log(`\n✅ Local database ready at ${DB_PATH} with ${count.n} exhibits`);

db.close();
