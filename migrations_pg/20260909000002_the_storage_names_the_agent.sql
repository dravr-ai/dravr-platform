-- ABOUTME: Renames the twelve coach-named tables and their agent-pointing columns to agent (PostgreSQL)
-- ABOUTME: A column referencing coaches(id) named the AI persona and moves; one referencing users(id) is a human and stays

-- ADR-026 reserves "coach" for a human professional and for the activity of
-- coaching. The AI persona is an agent. The tool names, routes and Rust types
-- moved in the two commits before this one; the storage they read still said
-- coach, which left one concept spelled two ways across a single query.
--
-- What moves is decided by the foreign key, not by the column's spelling: a
-- column that REFERENCES coaches(id) names the persona and is renamed, and a
-- column that REFERENCES users(id) names a person and is not. That is why
-- coach_athlete_assignments.coach_user_id and coaching_groups.coach_user_id
-- keep their names while coaching_groups.coach_id, one column over, becomes
-- agent_id. The three tables built around the activity -- coaching_groups,
-- coaching_group_members and coaching_playbooks -- keep their names for the
-- same reason: a group is coached, and a playbook is a pattern about coaching.
--
-- Columns carrying no foreign key were each read at their write site rather
-- than guessed, because guessing wrong here renames a person.
--
-- Renames are used rather than a copy-and-backfill so the rows, the indexes
-- and every foreign key pointing at them survive untouched: both backends
-- rewrite referencing constraints as part of the rename. Columns move first,
-- while their tables still answer to the old name; the tables follow; the
-- indexes are renamed last so their names describe what they now index.

ALTER TABLE athlete_commitments RENAME COLUMN coach_id TO agent_id;
ALTER TABLE chat_conversations RENAME COLUMN coach_id TO agent_id;
ALTER TABLE claim_verdicts RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_artefacts RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_assignments RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_authors RENAME COLUMN published_coach_count TO published_agent_count;
ALTER TABLE coach_followups RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_notes RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_relations RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_relations RENAME COLUMN related_coach_id TO related_agent_id;
ALTER TABLE coach_sessions RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_translations RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_versions RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coaching_groups RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coaching_playbooks RENAME COLUMN coach_slug TO agent_slug;
ALTER TABLE messaging_channel_links RENAME COLUMN coach_proposal_sent_at TO agent_proposal_sent_at;
ALTER TABLE messaging_channel_links RENAME COLUMN proposed_coach_ids TO proposed_agent_ids;
ALTER TABLE pending_advice RENAME COLUMN coach_slug TO agent_slug;
ALTER TABLE prescribed_workouts RENAME COLUMN coach_id TO agent_id;
ALTER TABLE store_listings RENAME COLUMN coach_id TO agent_id;
ALTER TABLE tenant_users RENAME COLUMN selected_coach_id TO selected_agent_id;
ALTER TABLE training_plans RENAME COLUMN coach_slug TO agent_slug;
ALTER TABLE usage_counters RENAME COLUMN coach_id TO agent_id;
ALTER TABLE user_coach_preferences RENAME COLUMN coach_id TO agent_id;
ALTER TABLE user_facts RENAME COLUMN coach_id TO agent_id;
ALTER TABLE coach_artefacts RENAME TO agent_artefacts;
ALTER TABLE coach_assignments RENAME TO agent_assignments;
ALTER TABLE coach_authors RENAME TO agent_authors;
ALTER TABLE coach_followups RENAME TO agent_followups;
ALTER TABLE coach_notes RENAME TO agent_notes;
ALTER TABLE coach_relations RENAME TO agent_relations;
ALTER TABLE coach_sessions RENAME TO agent_sessions;
ALTER TABLE coach_translations RENAME TO agent_translations;
ALTER TABLE coach_versions RENAME TO agent_versions;
ALTER TABLE coaches RENAME TO agents;
ALTER TABLE coaches_orphaned RENAME TO agents_orphaned;
ALTER TABLE user_coach_preferences RENAME TO user_agent_preferences;

ALTER INDEX IF EXISTS idx_chat_conversations_coach_id RENAME TO idx_chat_conversations_agent_id;
ALTER INDEX IF EXISTS idx_claim_verdicts_coach RENAME TO idx_claim_verdicts_agent;
ALTER INDEX IF EXISTS idx_coach_artefacts_coach RENAME TO idx_agent_artefacts_agent;
ALTER INDEX IF EXISTS idx_coach_assignments_coach RENAME TO idx_agent_assignments_agent;
ALTER INDEX IF EXISTS idx_coach_assignments_favorite RENAME TO idx_agent_assignments_favorite;
ALTER INDEX IF EXISTS idx_coach_assignments_user RENAME TO idx_agent_assignments_user;
ALTER INDEX IF EXISTS idx_coach_authors_popular RENAME TO idx_agent_authors_popular;
ALTER INDEX IF EXISTS idx_coach_authors_user RENAME TO idx_agent_authors_user;
ALTER INDEX IF EXISTS idx_coach_authors_verified RENAME TO idx_agent_authors_verified;
ALTER INDEX IF EXISTS idx_coach_followups_pending_due RENAME TO idx_agent_followups_pending_due;
ALTER INDEX IF EXISTS idx_coach_followups_tenant_user_coach RENAME TO idx_agent_followups_tenant_user_agent;
ALTER INDEX IF EXISTS idx_coach_notes_active_suppressed RENAME TO idx_agent_notes_active_suppressed;
ALTER INDEX IF EXISTS idx_coach_notes_created_at RENAME TO idx_agent_notes_created_at;
ALTER INDEX IF EXISTS idx_coach_notes_tenant_user_coach RENAME TO idx_agent_notes_tenant_user_agent;
ALTER INDEX IF EXISTS idx_coach_relations_coach RENAME TO idx_agent_relations_agent;
ALTER INDEX IF EXISTS idx_coach_relations_related RENAME TO idx_agent_relations_related;
ALTER INDEX IF EXISTS idx_coach_sessions_active_last_turn RENAME TO idx_agent_sessions_active_last_turn;
ALTER INDEX IF EXISTS idx_coach_sessions_tenant_user_coach RENAME TO idx_agent_sessions_tenant_user_agent;
ALTER INDEX IF EXISTS idx_coach_translations_locale RENAME TO idx_agent_translations_locale;
ALTER INDEX IF EXISTS idx_coach_versions_coach RENAME TO idx_agent_versions_agent;
ALTER INDEX IF EXISTS idx_coaches_category RENAME TO idx_agents_category;
ALTER INDEX IF EXISTS idx_coaches_forked_from RENAME TO idx_agents_forked_from;
ALTER INDEX IF EXISTS idx_coaches_handle RENAME TO idx_agents_handle;
ALTER INDEX IF EXISTS idx_coaches_handle_copy RENAME TO idx_agents_handle_copy;
ALTER INDEX IF EXISTS idx_coaches_source RENAME TO idx_agents_source;
ALTER INDEX IF EXISTS idx_coaches_source_file RENAME TO idx_agents_source_file;
ALTER INDEX IF EXISTS idx_coaches_system RENAME TO idx_agents_system;
ALTER INDEX IF EXISTS idx_coaches_tenant RENAME TO idx_agents_tenant;
ALTER INDEX IF EXISTS idx_coaches_user RENAME TO idx_agents_user;
ALTER INDEX IF EXISTS idx_coaching_groups_coach RENAME TO idx_coaching_groups_agent;
ALTER INDEX IF EXISTS idx_store_listings_coach_id RENAME TO idx_store_listings_agent_id;
ALTER INDEX IF EXISTS idx_tenant_users_selected_coach RENAME TO idx_tenant_users_selected_agent;
ALTER INDEX IF EXISTS idx_usage_counters_coach RENAME TO idx_usage_counters_agent;
ALTER INDEX IF EXISTS idx_user_coach_prefs_user RENAME TO idx_user_agent_prefs_user;
ALTER INDEX IF EXISTS idx_user_facts_tenant_user_coach RENAME TO idx_user_facts_tenant_user_agent;
