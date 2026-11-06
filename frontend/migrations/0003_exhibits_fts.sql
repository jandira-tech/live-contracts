-- Full-text index for /search and the MCP search_agreements tool.
-- A standalone FTS5 table (not external-content: `exhibits.id` is a text key, so
-- rowids are not stable enough to point at). Triggers keep it in step with every
-- INSERT / UPDATE / DELETE the ingest route performs. Backfill of existing rows
-- is a separate, chunked step: `bun run db:fts-backfill` (scripts/fts-backfill.ts).
CREATE VIRTUAL TABLE IF NOT EXISTS `exhibits_fts` USING fts5(
	`id` UNINDEXED,
	`description`,
	`markdown`,
	tokenize = 'unicode61 remove_diacritics 2'
);--> statement-breakpoint
CREATE TRIGGER IF NOT EXISTS `exhibits_fts_ai` AFTER INSERT ON `exhibits` BEGIN
	INSERT INTO `exhibits_fts`(`id`, `description`, `markdown`) VALUES (new.`id`, coalesce(new.`description`, ''), coalesce(new.`markdown`, ''));
END;--> statement-breakpoint
CREATE TRIGGER IF NOT EXISTS `exhibits_fts_ad` AFTER DELETE ON `exhibits` BEGIN
	DELETE FROM `exhibits_fts` WHERE `id` = old.`id`;
END;--> statement-breakpoint
CREATE TRIGGER IF NOT EXISTS `exhibits_fts_au` AFTER UPDATE OF `description`, `markdown` ON `exhibits` BEGIN
	DELETE FROM `exhibits_fts` WHERE `id` = old.`id`;
	INSERT INTO `exhibits_fts`(`id`, `description`, `markdown`) VALUES (new.`id`, coalesce(new.`description`, ''), coalesce(new.`markdown`, ''));
END;
