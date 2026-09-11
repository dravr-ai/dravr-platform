-- ABOUTME: Renames the twelve coach-named tables and their agent-pointing columns to agent (SQLite)
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
ALTER TABLE user_coach_preferences RENAME TO user_agent_preferences;

DROP INDEX IF EXISTS idx_chat_conversations_coach_id;
CREATE INDEX IF NOT EXISTS idx_chat_conversations_agent_id ON chat_conversations(agent_id) WHERE agent_id IS NOT NULL;
DROP INDEX IF EXISTS idx_claim_verdicts_coach;
CREATE INDEX IF NOT EXISTS idx_claim_verdicts_agent ON claim_verdicts(agent_id, created_at DESC) WHERE agent_id IS NOT NULL;
DROP INDEX IF EXISTS idx_coach_artefacts_coach;
CREATE INDEX IF NOT EXISTS idx_agent_artefacts_agent ON agent_artefacts(tenant_id, agent_id);
DROP INDEX IF EXISTS idx_coach_assignments_coach;
CREATE INDEX IF NOT EXISTS idx_agent_assignments_agent ON agent_assignments(agent_id);
DROP INDEX IF EXISTS idx_coach_assignments_favorite;
CREATE INDEX IF NOT EXISTS idx_agent_assignments_favorite ON agent_assignments(user_id, is_favorite) WHERE is_favorite = 1;
DROP INDEX IF EXISTS idx_coach_assignments_user;
CREATE INDEX IF NOT EXISTS idx_agent_assignments_user ON agent_assignments(user_id);
DROP INDEX IF EXISTS idx_coach_authors_popular;
CREATE INDEX IF NOT EXISTS idx_agent_authors_popular ON agent_authors(total_install_count DESC) WHERE published_agent_count > 0;
DROP INDEX IF EXISTS idx_coach_authors_user;
CREATE INDEX IF NOT EXISTS idx_agent_authors_user ON agent_authors(user_id, tenant_id);
DROP INDEX IF EXISTS idx_coach_authors_verified;
CREATE INDEX IF NOT EXISTS idx_agent_authors_verified ON agent_authors(is_verified, total_install_count DESC) WHERE is_verified = 1;
DROP INDEX IF EXISTS idx_coach_followups_pending_due;
CREATE INDEX IF NOT EXISTS idx_agent_followups_pending_due ON agent_followups(tenant_id, user_id, agent_id, due_at) WHERE status = 'pending';
DROP INDEX IF EXISTS idx_coach_followups_tenant_user_coach;
CREATE INDEX IF NOT EXISTS idx_agent_followups_tenant_user_agent ON agent_followups(tenant_id, user_id, agent_id);
DROP INDEX IF EXISTS idx_coach_notes_active_suppressed;
CREATE INDEX IF NOT EXISTS idx_agent_notes_active_suppressed ON agent_notes(tenant_id, user_id, agent_id, created_at DESC) WHERE suppressed = 0;
DROP INDEX IF EXISTS idx_coach_notes_created_at;
CREATE INDEX IF NOT EXISTS idx_agent_notes_created_at ON agent_notes(created_at DESC);
DROP INDEX IF EXISTS idx_coach_notes_tenant_user_coach;
CREATE INDEX IF NOT EXISTS idx_agent_notes_tenant_user_agent ON agent_notes(tenant_id, user_id, agent_id);
DROP INDEX IF EXISTS idx_coach_relations_coach;
CREATE INDEX IF NOT EXISTS idx_agent_relations_agent ON agent_relations(agent_id);
DROP INDEX IF EXISTS idx_coach_relations_related;
CREATE INDEX IF NOT EXISTS idx_agent_relations_related ON agent_relations(related_agent_id);
DROP INDEX IF EXISTS idx_coach_sessions_active_last_turn;
CREATE INDEX IF NOT EXISTS idx_agent_sessions_active_last_turn ON agent_sessions(tenant_id, user_id, last_turn_at DESC) WHERE status = 'active';
DROP INDEX IF EXISTS idx_coach_sessions_tenant_user_coach;
CREATE INDEX IF NOT EXISTS idx_agent_sessions_tenant_user_agent ON agent_sessions(tenant_id, user_id, agent_id);
DROP INDEX IF EXISTS idx_coach_translations_locale;
CREATE INDEX IF NOT EXISTS idx_agent_translations_locale ON agent_translations(locale);
DROP INDEX IF EXISTS idx_coach_versions_coach;
CREATE INDEX IF NOT EXISTS idx_agent_versions_agent ON agent_versions(agent_id, version DESC);
DROP INDEX IF EXISTS idx_coaches_category;
CREATE INDEX IF NOT EXISTS idx_agents_category ON agents(user_id, category);
DROP INDEX IF EXISTS idx_coaches_forked_from;
CREATE INDEX IF NOT EXISTS idx_agents_forked_from ON agents(forked_from) WHERE forked_from IS NOT NULL;
DROP INDEX IF EXISTS idx_coaches_handle;
CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_handle ON agents(slug) WHERE slug IS NOT NULL AND forked_from IS NULL;
DROP INDEX IF EXISTS idx_coaches_handle_copy;
CREATE INDEX IF NOT EXISTS idx_agents_handle_copy ON agents(slug, user_id) WHERE slug IS NOT NULL AND forked_from IS NOT NULL;
DROP INDEX IF EXISTS idx_coaches_source;
CREATE INDEX IF NOT EXISTS idx_agents_source ON agents(source);
DROP INDEX IF EXISTS idx_coaches_source_file;
CREATE INDEX IF NOT EXISTS idx_agents_source_file ON agents(source_file) WHERE source_file IS NOT NULL;
DROP INDEX IF EXISTS idx_coaches_startup_query;
CREATE INDEX IF NOT EXISTS idx_agents_startup_query ON agents(id) WHERE startup_query IS NOT NULL;
DROP INDEX IF EXISTS idx_coaches_system;
CREATE INDEX IF NOT EXISTS idx_agents_system ON agents(tenant_id, is_system, visibility) WHERE is_system = 1;
DROP INDEX IF EXISTS idx_coaches_tenant;
CREATE INDEX IF NOT EXISTS idx_agents_tenant ON agents(tenant_id);
DROP INDEX IF EXISTS idx_coaches_user;
CREATE INDEX IF NOT EXISTS idx_agents_user ON agents(user_id);
DROP INDEX IF EXISTS idx_coaching_groups_coach;
CREATE INDEX IF NOT EXISTS idx_coaching_groups_agent ON coaching_groups(agent_id);
DROP INDEX IF EXISTS idx_store_listings_coach_id;
CREATE INDEX IF NOT EXISTS idx_store_listings_agent_id ON store_listings(agent_id);
DROP INDEX IF EXISTS idx_tenant_users_selected_coach;
CREATE INDEX IF NOT EXISTS idx_tenant_users_selected_agent ON tenant_users(selected_agent_id);
DROP INDEX IF EXISTS idx_usage_counters_coach;
CREATE INDEX IF NOT EXISTS idx_usage_counters_agent ON usage_counters(tenant_id, user_id, agent_id, counter_key, period);
DROP INDEX IF EXISTS idx_user_coach_prefs_hidden;
CREATE INDEX IF NOT EXISTS idx_user_agent_prefs_hidden ON user_agent_preferences(user_id, is_hidden) WHERE is_hidden = 1;
DROP INDEX IF EXISTS idx_user_coach_prefs_user;
CREATE INDEX IF NOT EXISTS idx_user_agent_prefs_user ON user_agent_preferences(user_id);
DROP INDEX IF EXISTS idx_user_facts_tenant_user_coach;
CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user_agent ON user_facts(tenant_id, user_id, agent_id) WHERE agent_id IS NOT NULL;
