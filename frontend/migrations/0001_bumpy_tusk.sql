PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE `__new_exhibits` (
	`id` text PRIMARY KEY NOT NULL,
	`accession` text NOT NULL,
	`cik` text,
	`form_type` text,
	`doc_type` text,
	`filename` text NOT NULL,
	`description` text,
	`sequence` text,
	`filing_url` text,
	`found_at` text,
	`filed_at` text,
	`markdown_status` text,
	`filing_metadata` text,
	`image_urls` text,
	`markdown` text
);
--> statement-breakpoint
INSERT INTO `__new_exhibits`("id", "accession", "cik", "form_type", "doc_type", "filename", "description", "sequence", "filing_url", "found_at", "filed_at", "markdown_status", "filing_metadata", "image_urls", "markdown") SELECT "id", "accession", "cik", "form_type", "doc_type", "filename", "description", "sequence", "filing_url", "found_at", "filed_at", "markdown_status", "filing_metadata", "image_urls", "markdown" FROM `exhibits`;--> statement-breakpoint
DROP TABLE `exhibits`;--> statement-breakpoint
ALTER TABLE `__new_exhibits` RENAME TO `exhibits`;--> statement-breakpoint
PRAGMA foreign_keys=ON;--> statement-breakpoint
CREATE UNIQUE INDEX `uniq_acc_doc_file` ON `exhibits` (`accession`,`doc_type`,`filename`);--> statement-breakpoint
CREATE INDEX `idx_ex_filed_at` ON `exhibits` (`filed_at`);--> statement-breakpoint
CREATE INDEX `idx_ex_form_type` ON `exhibits` (`form_type`);--> statement-breakpoint
CREATE INDEX `idx_ex_cik` ON `exhibits` (`cik`);