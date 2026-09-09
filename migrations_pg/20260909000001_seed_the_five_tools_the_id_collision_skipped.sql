-- ABOUTME: Seeds the five tool_catalog rows that 20260907000003 silently skipped on an id collision
-- ABOUTME: Fresh ids tc-140..tc-144; the two applied migrations stay byte-identical so their checksums hold
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- 20260907000002 renames 26 persona tools and claims ids tc-109..tc-134.
-- 20260907000003 seeds five uncatalogued tools and claimed tc-109..tc-113 --
-- written first, numbered first, and now overlapping. 000002 runs earlier, so
-- by the time 000003 inserts, those five primary keys are taken and its
-- conflict clause drops all five rows without a word. The tools register at
-- runtime and get a synthesised `tc-auto-` row, so nothing looks broken until
-- `the_seeded_catalog_and_the_registry_name_the_same_tools` counts them.
--
-- The obvious repair -- renumber 000003 -- was made and reverted, because both
-- files are already applied: sqlx stores a checksum per migration and refuses
-- to start against a modified one, which took dev down twice. An applied
-- migration is immutable. So this adds a new one instead.
--
-- Idempotent on tool_name, not on id: a database that has already booted holds
-- a `tc-auto-` row under each of these names and keeps it, along with any text
-- an operator has edited. Only a database that never synthesised them -- a
-- fresh test database, which is what the assertion runs against -- takes these.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-140', 'estimate_vo2max', 'Estimate VO2max', 'Estimate VO2max from a field test the athlete describes', 'configuration', TRUE, NULL, 'starter'),
('tc-141', 'estimate_lactate_thresholds', 'Estimate Lactate Thresholds', 'Locate LT1 and LT2 from a step test the athlete reports', 'configuration', TRUE, NULL, 'starter'),
('tc-142', 'recommend_plan_flavour', 'Recommend Plan Flavour', 'Rank the training flavours an athlete can run and lay out the season', 'configuration', TRUE, NULL, 'starter'),
('tc-143', 'forget_playbook', 'Forget Playbook', 'Delete one learned coaching playbook by id (GDPR forget)', 'fitness', TRUE, NULL, 'starter'),
('tc-144', 'verify_claim', 'Verify Claim', 'Check a factual claim against the claim-verification pipeline', 'fitness', TRUE, NULL, 'starter')
ON CONFLICT (tool_name) DO NOTHING;
