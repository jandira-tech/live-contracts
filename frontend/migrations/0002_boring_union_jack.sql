ALTER TABLE `exhibits` ADD `source` text;--> statement-breakpoint
ALTER TABLE `exhibits` ADD `size_bytes` integer;--> statement-breakpoint
ALTER TABLE `exhibits` ADD `detected_at` text;--> statement-breakpoint
CREATE INDEX `idx_ex_detected_at` ON `exhibits` (`detected_at`);