-- ABOUTME: Seeds the five registered tools that reached the catalog only through the startup sync
-- ABOUTME: A seeded row is the reviewed artifact a rename must carry; a synthesised one is not
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- `sync_tool_catalog` (mcp/tool_selection.rs) runs at every server boot: it
-- inserts a `tc-auto-<uuid>` row for each registered tool the catalog lacks,
-- and deletes each catalog row naming a tool the registry no longer has. Five
-- tools reached the catalog only that way — estimate_vo2max,
-- estimate_lactate_thresholds and recommend_plan_flavour (registry category
-- `physiology`), forget_playbook (`playbook`) and verify_claim
-- (`verification`). Two things follow from having no seeded row, and this
-- migration is about both:
--
--   * Until the first boot, the tool has no catalog row at all, and
--     `guardian::tenant_tool_enabled` reads a missing row as
--     `ResourceNotFound` = "no per-tenant override applies" and allows. No
--     tenant can disable the tool in that window (carnet#143).
--   * A synthesised row carries `display_name_from_tool_name` output and the
--     tool's LLM-facing description verbatim as the text an operator reads in
--     the tool picker — for estimate_lactate_thresholds that is the full
--     700-character prompt written for a model, not for a person.
--
-- Every column below is what `sync_tool_catalog` derives for the same tool, so
-- a database seeded here and one that synthesised the row agree: none of the
-- five declares REQUIRES_PROVIDER or ADMIN_ONLY, so requires_provider is NULL
-- and is_enabled_by_default is 1, and min_plan is 'starter' — a higher plan
-- gate would REMOVE a tool every tenant can already call. `category_from_
-- registry` maps `physiology` to Configuration, and both `playbook` and
-- `verification` fall through to Fitness, so the catalog values are
-- 'configuration' and 'fitness'. No new category values, so the CHECK
-- constraint is untouched.
--
-- INSERT OR IGNORE: a database that has already booted holds a tc-auto row
-- under these tool_names and keeps it, along with any text an operator changed.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-109', 'estimate_vo2max', 'Estimate VO2max', 'Estimate VO2max from a field test the athlete describes', 'configuration', 1, NULL, 'starter'),
('tc-110', 'estimate_lactate_thresholds', 'Estimate Lactate Thresholds', 'Locate LT1 and LT2 from a step test the athlete reports', 'configuration', 1, NULL, 'starter'),
('tc-111', 'recommend_plan_flavour', 'Recommend Plan Flavour', 'Rank the training flavours an athlete can run and lay out the season', 'configuration', 1, NULL, 'starter'),
('tc-112', 'forget_playbook', 'Forget Playbook', 'Delete one learned coaching playbook by id (GDPR forget)', 'fitness', 1, NULL, 'starter'),
('tc-113', 'verify_claim', 'Verify Claim', 'Check a factual claim against the claim-verification pipeline', 'fitness', 1, NULL, 'starter');
