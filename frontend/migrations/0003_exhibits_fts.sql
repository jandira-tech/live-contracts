-- Full-text index for /search and the MCP search_agreements tool.
-- External content: the index stores tokens and document sizes only and reads
-- `description` / `markdown` back from `exhibits` by rowid, so the 1.8 GB of
-- markdown is not stored twice. `exhibits.id` is a TEXT key, so the implicit
-- rowid is the join key: never join FTS on the UUID.
-- A migration that rebuilds `exhibits` (create/copy/drop/rename) renumbers rowids
-- and drops these triggers: recreate them and run
--   INSERT INTO exhibits_fts(exhibits_fts) VALUES('rebuild');
-- Backfill rows that predate the triggers with `pnpm db:fts-backfill` or a single
-- `rebuild`, then verify with
--   INSERT INTO exhibits_fts(exhibits_fts, rank) VALUES('integrity-check', 1);
-- Deliberately unguarded: a database still holding the old standalone table must fail here.
CREATE VIRTUAL TABLE `exhibits_fts` USING fts5(
	`description`,
	`markdown`,
	content = 'exhibits',
	content_rowid = 'rowid',
	tokenize = 'unicode61 remove_diacritics 2'
);--> statement-breakpoint
CREATE TRIGGER `exhibits_fts_ai` AFTER INSERT ON `exhibits` BEGIN
	INSERT INTO `exhibits_fts`(`rowid`, `description`, `markdown`)
	VALUES (new.`rowid`, coalesce(new.`description`, ''), coalesce(new.`markdown`, ''));
END;--> statement-breakpoint
CREATE TRIGGER `exhibits_fts_ad` AFTER DELETE ON `exhibits` BEGIN
	INSERT INTO `exhibits_fts`(`exhibits_fts`, `rowid`, `description`, `markdown`)
	VALUES ('delete', old.`rowid`, coalesce(old.`description`, ''), coalesce(old.`markdown`, ''));
END;--> statement-breakpoint
-- Ingest rewrites description and markdown on every conflict; the IS NOT guard
-- (NULL-safe) skips re-tokenizing an unchanged ~94 KB body on each re-push.
CREATE TRIGGER `exhibits_fts_au` AFTER UPDATE OF `description`, `markdown` ON `exhibits`
WHEN new.`description` IS NOT old.`description` OR new.`markdown` IS NOT old.`markdown` BEGIN
	INSERT INTO `exhibits_fts`(`exhibits_fts`, `rowid`, `description`, `markdown`)
	VALUES ('delete', old.`rowid`, coalesce(old.`description`, ''), coalesce(old.`markdown`, ''));
	INSERT INTO `exhibits_fts`(`rowid`, `description`, `markdown`)
	VALUES (new.`rowid`, coalesce(new.`description`, ''), coalesce(new.`markdown`, ''));
END;
