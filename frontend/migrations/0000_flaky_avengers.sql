CREATE TABLE `exhibits` (
	`id` integer PRIMARY KEY NOT NULL,
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
CREATE UNIQUE INDEX `uniq_acc_doc_file` ON `exhibits` (`accession`,`doc_type`,`filename`);--> statement-breakpoint
CREATE INDEX `idx_ex_filed_at` ON `exhibits` (`filed_at`);--> statement-breakpoint
CREATE INDEX `idx_ex_form_type` ON `exhibits` (`form_type`);--> statement-breakpoint
CREATE INDEX `idx_ex_cik` ON `exhibits` (`cik`);