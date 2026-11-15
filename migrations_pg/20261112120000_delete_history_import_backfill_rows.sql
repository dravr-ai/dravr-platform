-- ABOUTME: Deletes the backfill ledger rows only the retired one-time Garmin history import wrote (carnet#1494)
-- ABOUTME: Its owed imports and the incomplete rows it kept go; the kind and outcome columns stay, unread

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Sciotte first (carnet#1494) makes a usr/pwd session the provider's primary
-- source, so a session beside a grant is never imported once and retired any
-- more (carnet#1386). The build that lands with this no longer reads the
-- `kind` and `outcome` columns 20261106120000 added for that import: a row
-- still owed as an import ('history_import') would resume as an ordinary
-- whole-account backfill of a session the import may already have retired,
-- and a row kept as 'incomplete' would block nothing yet owe nothing. Both are
-- deleted. Ordinary rows (kind and outcome NULL) are untouched. The columns
-- themselves stay: dropping one makes a rollback deploy unsafe.
DELETE FROM activity_backfill_jobs WHERE kind IS NOT NULL OR outcome IS NOT NULL;
