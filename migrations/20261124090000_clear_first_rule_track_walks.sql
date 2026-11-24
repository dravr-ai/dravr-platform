-- ABOUTME: Deletes the track walk rows of the backfill ledger, so each session connection is walked once under the new rule (carnet#1779)
-- ABOUTME: Only the 'track_walk' kind; the next head refresh asks a fresh walk that reads the rides no detail read reached

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The track walk (carnet#1679) read only the rides a detail read had reached,
-- so a ride no detail read ever reached (a session list row carries no route,
-- and a detail walk reads only the window it is asked for) never got a track
-- (carnet#1779). The build that lands with this reads every outdoor ride whose
-- track is unsettled, and asks a track walk once per session connection: a
-- connection whose walk is owed or walked is never asked again. Its rows are
-- deleted so the next head refresh asks a fresh walk under the new rule. A
-- ride the old walk read holds a stored track or a settled-absent route and
-- is no candidate again; only the few the backend could not read are read
-- once more. Ledger rows of this one kind only, in every tenant: no athlete
-- data, and no other walk kind, is touched.
DELETE FROM activity_backfill_jobs WHERE kind = 'track_walk';
