-- ABOUTME: Seeds tool_catalog rows for the 29 chat-callable tools missing from the catalog
-- ABOUTME: Uncatalogued tools cannot be disabled per tenant, so the plan-scope guard was inert (carnet#143)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- `guardian::tenant_tool_enabled` treats an uncatalogued tool as always
-- enabled (ResourceNotFound = "no per-tenant override applies"), so no tenant
-- could disable get_training_plan / save_training_plan / push_training_plan —
-- the cross-tenant coach-write check in plan_scope.rs could never refuse in
-- production. This seeds every chat-callable tool that had no catalog row
-- (the chat_callable_schemas category allowlist × register_builtin_tools,
-- pinned by tool_catalog_completeness_test.rs).
--
-- Every row is is_enabled_by_default = 1 and min_plan = 'starter': these
-- tools were always-on for every tenant while uncatalogued, and a higher
-- plan gate would REMOVE a tool from tenants that could already call it
-- (the plan check runs before tenant overrides — see the 20260711 sleep
-- alignment). requires_provider stays NULL like every comparable row.
-- Categories map the registry's taxonomy into the existing catalog CHECK
-- vocabulary: data → fitness, analytics → analysis, connection → connections,
-- physiology → configuration, memory/store → coaches, commitments → goals,
-- groups → fitness. No new category values, so the CHECK constraint is
-- untouched. INSERT OR IGNORE: never touches an existing row.

-- Connection: sync/refresh tools (registry category `connection`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-080', 'refresh_provider_data', 'Refresh Provider Data', 'Trigger a data refresh from a connected fitness provider', 'connections', 1, NULL, 'starter'),
('tc-081', 'get_data_freshness', 'Data Freshness', 'Check how fresh fitness data is across all connected providers', 'connections', 1, NULL, 'starter');

-- Endurance data tools (registry category `data`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-082', 'export_latest_snapshot', 'Export Latest Snapshot', 'Export the Endurance latest.json snapshot for the authenticated user', 'fitness', 1, NULL, 'starter'),
('tc-083', 'export_dossier', 'Export Dossier', 'Export the Endurance dossier.json aggregate for the authenticated user', 'fitness', 1, NULL, 'starter'),
('tc-084', 'get_training_history', 'Training History', 'Fetch persisted Endurance daily training-state rollups', 'fitness', 1, NULL, 'starter'),
('tc-085', 'compute_training_history', 'Compute Training History', 'Compute and persist Endurance daily training-state rollups', 'fitness', 1, NULL, 'starter'),
('tc-086', 'export_intervals', 'Export Intervals', 'Export the Endurance intervals.json shape for a single activity', 'fitness', 1, NULL, 'starter'),
('tc-087', 'export_routes', 'Export Routes', 'Export the Endurance routes.json shape for a single activity', 'fitness', 1, NULL, 'starter'),
('tc-088', 'extract_activity_streams', 'Activity Streams', 'Return the raw per-second time-series streams for a single activity', 'fitness', 1, NULL, 'starter'),
('tc-089', 'list_workout_templates', 'Workout Templates', 'List the Endurance cornerstone workout templates', 'fitness', 1, NULL, 'starter'),
('tc-090', 'prescribe_workout', 'Prescribe Workout', 'Write one workout onto the athlete''s provider calendar', 'fitness', 1, NULL, 'starter'),
('tc-091', 'withdraw_prescribed_workout', 'Withdraw Prescribed Workout', 'Remove a workout that prescribe_workout wrote to the athlete''s calendar', 'fitness', 1, NULL, 'starter');

-- Route discovery and weather forecast (registry category `analytics`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-092', 'discover_routes', 'Discover Routes', 'Discover real named running, cycling, hiking, or ski routes near a location', 'analysis', 1, NULL, 'starter'),
('tc-093', 'get_weather_forecast', 'Weather Forecast', 'Get the weather forecast for a location to plan upcoming sessions', 'analysis', 1, NULL, 'starter');

-- Consent-gated peer activity fetch (registry category `groups`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-094', 'get_group_member_activities', 'Group Member Activities', 'Fetch a consenting group member''s recent or past activities', 'fitness', 1, NULL, 'starter');

-- Athlete self-reported physiology (registry category `physiology`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-095', 'set_physiology', 'Set Physiology', 'Save the athlete''s physiological measurements (FTP, thresholds, heart rate, weight)', 'configuration', 1, NULL, 'starter');

-- Coach-authored memory and playbook transparency (registry category `memory`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-096', 'remember_fact', 'Remember Fact', 'Persist a structured durable fact about the user', 'coaches', 1, NULL, 'starter'),
('tc-097', 'recall_user_memory', 'Recall User Memory', 'Retrieve stored facts the coach has remembered about the user', 'coaches', 1, NULL, 'starter'),
('tc-098', 'coach_note_add', 'Add Coach Note', 'Persist a private coach note about the user', 'coaches', 1, NULL, 'starter'),
('tc-099', 'coach_followup_schedule', 'Schedule Follow-up', 'Schedule a future check-in the coach should remember', 'coaches', 1, NULL, 'starter'),
('tc-100', 'list_coaching_playbooks', 'List Coaching Playbooks', 'List the coaching playbooks learned for this athlete', 'coaches', 1, NULL, 'starter');

-- Training-plan persistence and calendar push (registry category `memory`;
-- catalog `fitness` — plans are training data, and the plan_scope guard reads
-- exactly these rows under the ATHLETE's tenant before a cross-tenant write)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-101', 'get_training_plan', 'Get Training Plan', 'Fetch the athlete''s active training plan', 'fitness', 1, NULL, 'starter'),
('tc-102', 'save_training_plan', 'Save Training Plan', 'Persist the training plan agreed with the athlete', 'fitness', 1, NULL, 'starter'),
('tc-103', 'push_training_plan', 'Push Training Plan', 'Put the athlete''s active training plan on their provider calendar', 'fitness', 1, NULL, 'starter');

-- Athlete commitments (registry category `memory`; catalog `goals`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-104', 'commitment_create', 'Create Commitment', 'Record a commitment the athlete just made', 'goals', 1, NULL, 'starter'),
('tc-105', 'commitment_cancel', 'Cancel Commitment', 'Retract a commitment the athlete no longer wants to be held to', 'goals', 1, NULL, 'starter');

-- Coach Store browse / search / install (registry category `store`)
INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-106', 'browse_coach_store', 'Browse Coach Store', 'Browse the catalogue of published coaches anyone can install', 'coaches', 1, NULL, 'starter'),
('tc-107', 'search_coach_store', 'Search Coach Store', 'Search the Coach Store for published coaches', 'coaches', 1, NULL, 'starter'),
('tc-108', 'install_coach_from_store', 'Install Coach from Store', 'Install a published Coach Store coach into the athlete''s own library', 'coaches', 1, NULL, 'starter');
