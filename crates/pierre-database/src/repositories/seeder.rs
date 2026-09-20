// ABOUTME: Repository trait and shared statements for the seed-only operations domain
// ABOUTME: One SQL text per operation; the per-backend codecs and the body that binds them live in seeder_body.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The seeder, written once.
//!
//! The trait and every statement are here; the per-backend seed codecs and
//! the body that binds them are in [`super::seeder_body`].
//!
//! Three things the two backends store differently reach the shared body as
//! macro arguments rather than as two copies of the statements:
//!
//! - **uuid columns** (`users.id` and every column referencing it,
//!   `tenants.id`, `tenant_users.*`, `agents.user_id`/`tenant_id`): a
//!   [`super::uuid_columns`] codec binds and reads them, hyphenated text on
//!   `SQLite` and a native uuid on Postgres;
//! - **the usage tables' primary key**: `api_key_usage.id` and `a2a_usage.id`
//!   are `TEXT PRIMARY KEY` with no default on `SQLite`, so the seed's uuid is
//!   stored, and `SERIAL` on Postgres, so the table mints one and the seed's
//!   id is not sent. The id column and its placeholder are two macro
//!   literals and the bind goes through the seed codec;
//! - **`a2a_clients.capabilities`**: `TEXT` holding a JSON list on `SQLite`,
//!   `TEXT[]` on Postgres. The seed carries the list as JSON; the seed codec
//!   binds the checked text on one and the parsed list on the other.
//!
//! Timestamps bind as [`DateTime<Utc>`] on both: sqlx-sqlite encodes one as
//! `to_rfc3339_opts(AutoSi, false)`, the bytes `to_rfc3339()` produces, so the
//! `SQLite` TEXT columns hold what they always held. `TRUE`/`FALSE` are the
//! boolean spellings both engines accept (`SQLite` reads them as 1/0), and
//! `ON CONFLICT ... DO UPDATE` / `DO NOTHING` replace `INSERT OR REPLACE` /
//! `OR IGNORE`, which only `SQLite` spelled.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};
use pierre_core::models::User;
use uuid::Uuid;

use crate::seed_models::SeedAgentTranslation;
use crate::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedAgent, SeedAgentAuthor, SeedAgentRelation, SeedApiKey,
    SeedApiKeyUsage, SeedDemoUser, SeedLlmUsageRecord, SeedProviderConnection, SeedStoreListing,
    SeedSyntheticActivity, SeedTenant,
};

/// Tables that seeders are allowed to reset (prevent arbitrary table access)
#[derive(Debug, Clone, Copy)]
pub enum SeedTable {
    /// `users` table
    Users,
    /// `api_keys` table
    ApiKeys,
    /// `a2a_clients` table
    A2AClients,
    /// `llm_usage` table
    LlmUsage,
    /// `api_key_usage` table
    ApiKeyUsage,
    /// `a2a_usage` table
    A2AUsage,
    /// `synthetic_activities` table
    SyntheticActivities,
    /// `stretching_exercises` table
    StretchingExercises,
    /// `yoga_poses` table
    YogaPoses,
    /// `activity_muscle_mapping` table
    ActivityMuscleMapping,
}

impl SeedTable {
    /// Get the SQL table name
    #[must_use]
    pub const fn table_name(&self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::ApiKeys => "api_keys",
            Self::A2AClients => "a2a_clients",
            Self::LlmUsage => "llm_usage",
            Self::ApiKeyUsage => "api_key_usage",
            Self::A2AUsage => "a2a_usage",
            Self::SyntheticActivities => "synthetic_activities",
            Self::StretchingExercises => "stretching_exercises",
            Self::YogaPoses => "yoga_poses",
            Self::ActivityMuscleMapping => "activity_muscle_mapping",
        }
    }
}

/// Repository trait for seed-only database operations.
///
/// Used by seeder binaries to populate demo/test data.
/// Not used by the main server application. Provides write operations
/// for tables that only have read-only repository traits in the main app.
#[async_trait]
pub trait SeederRepository: Send + Sync {
    // ---- Generic operations ----

    /// Delete all rows from a seed table
    async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64>;

    /// Count rows in a seed table
    async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64>;

    // ---- User lookup (shared across seeders) ----

    /// Get the first admin user (`super_admin` or admin role)
    async fn seed_get_admin_user(&self) -> AppResult<Option<User>>;

    /// Get the `tenant_id` for a user
    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>>;

    /// Find a user by email address (returns full User if found)
    async fn seed_find_user_by_email(&self, email: &str) -> AppResult<Option<User>>;

    /// Get IDs of all non-admin users ordered by creation date
    async fn seed_get_non_admin_user_ids(&self) -> AppResult<Vec<Uuid>>;

    /// Count non-admin users
    async fn seed_count_non_admin_users(&self) -> AppResult<i64>;

    // ---- Mobility seeder (stretching, yoga, activity mappings) ----

    /// Upsert a stretching exercise (insert or replace on conflict)
    async fn seed_upsert_stretching_exercise(&self, exercise: &StretchingExercise)
        -> AppResult<()>;

    /// Upsert a yoga pose (insert or replace on conflict)
    async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()>;

    /// Upsert an activity-muscle mapping (insert or replace on conflict)
    async fn seed_upsert_activity_mapping(&self, mapping: &ActivityMuscleMapping) -> AppResult<()>;

    // ---- LLM usage seeder ----

    /// Delete LLM usage records for a specific tenant
    async fn seed_delete_llm_usage_by_tenant(&self, tenant_id: Uuid) -> AppResult<u64>;

    /// Insert a single LLM usage record
    async fn seed_insert_llm_usage(&self, record: &SeedLlmUsageRecord) -> AppResult<()>;

    // ---- Synthetic activities seeder ----

    /// Delete synthetic activities for a specific user
    async fn seed_delete_synthetic_by_user(&self, user_id: Uuid) -> AppResult<u64>;

    /// Insert a synthetic activity record
    async fn seed_insert_synthetic_activity(
        &self,
        activity: &SeedSyntheticActivity,
    ) -> AppResult<()>;

    /// Upsert a provider connection (insert or update on conflict)
    async fn seed_upsert_provider_connection(&self, conn: &SeedProviderConnection)
        -> AppResult<()>;

    // ---- Demo data seeder ----

    /// Check if a user exists by email, returning their ID if found
    async fn seed_check_user_exists(&self, email: &str) -> AppResult<Option<Uuid>>;

    /// Insert a demo user row
    async fn seed_insert_demo_user(&self, user: &SeedDemoUser) -> AppResult<()>;

    /// Insert a tenant row
    async fn seed_insert_tenant(&self, tenant: &SeedTenant) -> AppResult<()>;

    /// Insert a tenant-user junction row
    async fn seed_insert_tenant_user(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Update the `tenant_id` column on a user row
    async fn seed_update_user_tenant(&self, user_id: Uuid, tenant_id: Uuid) -> AppResult<()>;

    /// Check if an API key exists by name, returning its ID if found
    async fn seed_check_api_key_by_name(&self, name: &str) -> AppResult<Option<Uuid>>;

    /// Insert an API key
    async fn seed_insert_api_key(&self, key: &SeedApiKey) -> AppResult<()>;

    /// Check if an A2A client exists by name, returning its ID if found
    async fn seed_check_a2a_client_by_name(&self, name: &str) -> AppResult<Option<Uuid>>;

    /// Insert an A2A client
    async fn seed_insert_a2a_client(&self, client: &SeedA2AClient) -> AppResult<()>;

    /// Insert an API key usage record
    async fn seed_insert_api_key_usage(&self, usage: &SeedApiKeyUsage) -> AppResult<()>;

    /// Insert an A2A usage record
    async fn seed_insert_a2a_usage(&self, usage: &SeedA2AUsage) -> AppResult<()>;

    // ---- Agent seeder ----

    /// Find an agent by slug and tenant, returning `(id, content_hash)` if found
    async fn seed_find_agent_by_slug(
        &self,
        slug: &str,
        tenant_id: &str,
    ) -> AppResult<Option<(String, Option<String>)>>;

    /// Look up `(source, content_hash)` for an agent by slug, tenant-agnostic.
    ///
    /// Used by `pierre-cli check-drift agents` (the daily contremaitre→DB
    /// drift gate); not used by the seed write path. Returns `None` when
    /// no row matches the slug.
    async fn seed_find_agent_drift_info(
        &self,
        slug: &str,
    ) -> AppResult<Option<(String, Option<String>)>>;

    /// Insert an agent record
    async fn seed_insert_agent(&self, agent: &SeedAgent) -> AppResult<()>;

    /// Update an existing agent record
    async fn seed_update_agent(&self, agent: &SeedAgent) -> AppResult<()>;

    /// Insert an agent relation if it doesn't already exist, returns true if inserted
    async fn seed_insert_agent_relation_if_absent(
        &self,
        relation: &SeedAgentRelation,
    ) -> AppResult<bool>;

    /// Upsert an agent author profile, returning the author ID
    ///
    /// Creates the `agent_authors` row if absent (idempotent by `user_id + tenant_id` unique).
    /// Returns the `agent_authors.id` for use as `store_listings.author_id`.
    async fn seed_upsert_agent_author(&self, author: &SeedAgentAuthor) -> AppResult<String>;

    /// Insert a store listing if it doesn't already exist, returns true if inserted
    async fn seed_insert_store_listing_if_absent(
        &self,
        listing: &SeedStoreListing,
    ) -> AppResult<bool>;

    /// Upsert an `agent_translations` row for `(agent_id, locale)`.
    ///
    /// Replaces existing translation content so re-running the seeder after a
    /// file edit brings the row back in sync. `source_sha` captures the first
    /// 16 hex chars of `sha256(en.md)` at translation time; the loader uses it
    /// later to detect drift when English content changes.
    async fn seed_upsert_agent_translation(
        &self,
        translation: &SeedAgentTranslation,
    ) -> AppResult<()>;

    /// List every catalogue-owned system agent in a tenant as `(id, slug)`.
    ///
    /// Catalogue-owned means the row was written from an agent markdown file:
    /// `source = 'contremaitre'`, or the transitional `'seed'` that the
    /// source-column migration stamped on rows seeded before it existed and
    /// that the seeder only re-stamps once the content hash changes.
    /// Operator-authored system agents (`source = 'custom'`) are never
    /// listed. The agent seeder diffs this against the slugs it discovered on
    /// disk and deletes the rest, so an agent retired from dravr-contremaitre
    /// leaves every database instead of lingering in the store.
    async fn seed_list_catalogue_agents(&self, tenant_id: &str)
        -> AppResult<Vec<(String, String)>>;

    /// Hand every live reference to a retired agent to its successor.
    ///
    /// Rewrites each statement in [`AGENT_POINTER_REWRITES`] and
    /// [`AGENT_POINTER_MERGES`] from `retired_agent_id` to
    /// `successor_agent_id`, so an athlete bound to a merged agent continues
    /// with the agent that absorbed it. Rows the agent owns outright
    /// (versions, relations, translations, listing, notes, follow-ups,
    /// sessions) are not moved; they cascade with the delete. Returns the
    /// number of rows rewritten.
    async fn seed_repoint_agent_references(
        &self,
        retired_agent_id: &str,
        successor_agent_id: &str,
    ) -> AppResult<u64>;

    /// Hand the slug-keyed references over as well.
    ///
    /// Separate from [`Self::seed_repoint_agent_references`] because the
    /// tables in [`AGENT_SLUG_REWRITES`] name the agent by slug rather than
    /// by id, so the caller has to supply both slugs. Returns the number of
    /// rows rewritten.
    async fn seed_repoint_agent_slug_references(
        &self,
        retired_slug: &str,
        successor_slug: &str,
    ) -> AppResult<u64>;

    /// Detach conversations from a retired agent that has no successor.
    ///
    /// `chat_conversations.agent_id` carries no `ON DELETE` rule, so a
    /// conversation still bound to the agent would block the delete; nulling
    /// it drops that conversation to the default prompt. Groups are left
    /// alone — a group needs an agent, so a group bound to an agent with no
    /// successor keeps blocking the delete until an operator picks one.
    /// Returns the number of conversations detached.
    async fn seed_detach_agent_conversations(&self, retired_agent_id: &str) -> AppResult<u64>;

    /// Stamp `source = 'contremaitre'` on the tenant's system agents still carrying `'seed'`.
    ///
    /// The source-column migration stamped the transitional `'seed'` on every
    /// row seeded before it existed, and the update path only re-stamps a row
    /// whose content hash changed — so an agent untouched since then stayed
    /// `'seed'` and the daily drift gate warned about it every morning. Every
    /// agent-seeder run claims those rows outright: a catalogue file for the
    /// slug is what makes the catalogue authoritative, not an edit. Returns
    /// the number of rows stamped.
    async fn seed_take_catalogue_ownership(&self, tenant_id: &str) -> AppResult<u64>;

    /// Every slug the catalogue owns in any tenant, for the drift gate.
    ///
    /// Tenant-agnostic like [`Self::seed_find_agent_drift_info`]: the gate
    /// walks the contremaitre checkout and compares, and a catalogue-owned
    /// row whose file is gone is the orphan it reports.
    async fn seed_list_catalogue_slugs(&self) -> AppResult<Vec<String>>;
}

/// What "catalogue-owned" means, as a SQL predicate.
///
/// `'contremaitre'` is the source the seeder stamps today and `'seed'` the one
/// it claims from before that stamp existed; a row outside the pair is an
/// operator's or an athlete's own agent and no seeder query may touch it.
/// Every catalogue query spells the same pair, so it is spelled once, as a
/// literal the statement consts below concatenate.
macro_rules! catalogue_source_filter {
    () => {
        "source IN ('contremaitre', 'seed')"
    };
}

/// One `UPDATE` per athlete-side pointer onto `agents.id`.
///
/// Identical on both engines: `$1` is the successor, `$2` the retired agent.
/// The tables an agent owns outright are deliberately absent — they cascade
/// with the row. `athlete_commitments.agent_id` carries no foreign key, so
/// nothing would cascade and nothing would error; an untouched row would
/// simply point at an id no row has any more.
pub const AGENT_POINTER_REWRITES: [&str; 6] = [
    "UPDATE chat_conversations SET agent_id = $1 WHERE agent_id = $2",
    "UPDATE coaching_groups SET agent_id = $1 WHERE agent_id = $2",
    "UPDATE tenant_users SET selected_agent_id = $1 WHERE selected_agent_id = $2",
    "UPDATE user_facts SET agent_id = $1 WHERE agent_id = $2",
    "UPDATE claim_verdicts SET agent_id = $1 WHERE agent_id = $2",
    "UPDATE athlete_commitments SET agent_id = $1 WHERE agent_id = $2",
];

/// The same rewrite for the two tables that hold one row per (athlete, agent).
///
/// `agent_assignments` is the install and `user_agent_preferences` the
/// hide/show flag; both carry a `UNIQUE` over the pair, so an athlete who
/// holds the retired agent *and* its successor already has the successor's
/// row and a plain `UPDATE` would collide on it. The `NOT EXISTS` leaves that
/// athlete's row where it is, and the `ON DELETE CASCADE` then drops it with
/// the retired agent — which is the right end state: they keep one install,
/// not two. Correlated against the target table, so it reads the same on
/// `SQLite` and `PostgreSQL`.
pub const AGENT_POINTER_MERGES: [&str; 2] = [
    "UPDATE agent_assignments SET agent_id = $1 WHERE agent_id = $2 \
     AND NOT EXISTS (SELECT 1 FROM agent_assignments held \
                     WHERE held.user_id = agent_assignments.user_id AND held.agent_id = $1)",
    "UPDATE user_agent_preferences SET agent_id = $1 WHERE agent_id = $2 \
     AND NOT EXISTS (SELECT 1 FROM user_agent_preferences held \
                     WHERE held.user_id = user_agent_preferences.user_id AND held.agent_id = $1)",
];

/// Bring the successor's published install counter back in line with its rows.
///
/// `store_listings.install_count` is a counter the install and uninstall
/// paths keep, not a figure read off `agent_assignments` — and it is what the
/// store screen prints to the athlete as "N installs". Handing installs over
/// moves the rows without touching it, so the successor would advertise only
/// the installs it held before the merge. Counting the rows also re-truths a
/// listing whose counter had already drifted. `$1` is the successor.
///
/// (`agents` carries an `install_count` column of its own from the original
/// store migration, but nothing has ever written or read it; the listing's is
/// the live one.)
pub const AGENT_INSTALL_COUNT_RESYNC: &str = "UPDATE store_listings SET install_count = \
     (SELECT COUNT(*) FROM agent_assignments WHERE agent_id = $1) WHERE agent_id = $1";

/// One `UPDATE` per athlete-side pointer that names the agent by *slug*.
///
/// These predate the id-keyed pointers and none of them carries a foreign
/// key, so a retired slug leaves them pointing at a name the catalogue no
/// longer knows: the athlete's learned playbooks, the advice waiting to be
/// delivered, the training plan that was built for them, and the workouts
/// already pushed to their calendar. `$1` is the successor's slug, `$2` the
/// retired one.
///
/// `prescribed_workouts.agent_id` is spelled `agent_id` but holds a slug:
/// both writers pass one (`plan_calendar_push` the plan's `agent_slug`,
/// `endurance_workouts` the turn's resolved slug) into a `TEXT` column. It
/// belongs here rather than with the id-keyed rewrites, where `$2` is a
/// `Uuid` and would match nothing.
pub const AGENT_SLUG_REWRITES: [&str; 4] = [
    "UPDATE coaching_playbooks SET agent_slug = $1 WHERE agent_slug = $2",
    "UPDATE pending_advice SET agent_slug = $1 WHERE agent_slug = $2",
    "UPDATE training_plans SET agent_slug = $1 WHERE agent_slug = $2",
    "UPDATE prescribed_workouts SET agent_id = $1 WHERE agent_id = $2",
];

// ============================================================================
// Statements
// ============================================================================

/// Insert or refresh a stretching exercise; the row keeps its `created_at`.
pub(crate) const UPSERT_STRETCHING_EXERCISE_SQL: &str = "INSERT INTO stretching_exercises \
             (id, name, description, category, difficulty, primary_muscles, secondary_muscles, \
              duration_seconds, repetitions, sets, recommended_for_activities, contraindications, \
              instructions, cues, image_url, video_url, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) \
             ON CONFLICT (id) DO UPDATE SET \
              name = EXCLUDED.name, description = EXCLUDED.description, \
              category = EXCLUDED.category, difficulty = EXCLUDED.difficulty, \
              primary_muscles = EXCLUDED.primary_muscles, secondary_muscles = EXCLUDED.secondary_muscles, \
              duration_seconds = EXCLUDED.duration_seconds, repetitions = EXCLUDED.repetitions, \
              sets = EXCLUDED.sets, recommended_for_activities = EXCLUDED.recommended_for_activities, \
              contraindications = EXCLUDED.contraindications, instructions = EXCLUDED.instructions, \
              cues = EXCLUDED.cues, image_url = EXCLUDED.image_url, video_url = EXCLUDED.video_url, \
              updated_at = EXCLUDED.updated_at";

/// Insert or refresh a yoga pose; the row keeps its `created_at`.
pub(crate) const UPSERT_YOGA_POSE_SQL: &str = "INSERT INTO yoga_poses \
             (id, english_name, sanskrit_name, description, benefits, \
              category, difficulty, pose_type, primary_muscles, secondary_muscles, \
              chakras, hold_duration_seconds, breath_guidance, \
              recommended_for_activities, recommended_for_recovery, contraindications, \
              instructions, modifications, progressions, cues, \
              warmup_poses, followup_poses, image_url, video_url, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                     $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, \
                     $21, $22, $23, $24, $25, $26) \
             ON CONFLICT (id) DO UPDATE SET \
              english_name = EXCLUDED.english_name, sanskrit_name = EXCLUDED.sanskrit_name, \
              description = EXCLUDED.description, benefits = EXCLUDED.benefits, \
              category = EXCLUDED.category, difficulty = EXCLUDED.difficulty, \
              pose_type = EXCLUDED.pose_type, primary_muscles = EXCLUDED.primary_muscles, \
              secondary_muscles = EXCLUDED.secondary_muscles, chakras = EXCLUDED.chakras, \
              hold_duration_seconds = EXCLUDED.hold_duration_seconds, \
              breath_guidance = EXCLUDED.breath_guidance, \
              recommended_for_activities = EXCLUDED.recommended_for_activities, \
              recommended_for_recovery = EXCLUDED.recommended_for_recovery, \
              contraindications = EXCLUDED.contraindications, \
              instructions = EXCLUDED.instructions, modifications = EXCLUDED.modifications, \
              progressions = EXCLUDED.progressions, cues = EXCLUDED.cues, \
              warmup_poses = EXCLUDED.warmup_poses, followup_poses = EXCLUDED.followup_poses, \
              image_url = EXCLUDED.image_url, video_url = EXCLUDED.video_url, \
              updated_at = EXCLUDED.updated_at";

/// Insert or refresh an activity's muscle mapping, keyed on the UNIQUE
/// `activity_type`; the row keeps its `created_at`.
pub(crate) const UPSERT_ACTIVITY_MAPPING_SQL: &str = "INSERT INTO activity_muscle_mapping \
             (id, activity_type, primary_muscles, secondary_muscles, \
              recommended_stretch_categories, recommended_yoga_categories, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (activity_type) DO UPDATE SET \
              primary_muscles = EXCLUDED.primary_muscles, \
              secondary_muscles = EXCLUDED.secondary_muscles, \
              recommended_stretch_categories = EXCLUDED.recommended_stretch_categories, \
              recommended_yoga_categories = EXCLUDED.recommended_yoga_categories, \
              updated_at = EXCLUDED.updated_at";

/// The email of the oldest admin account; the full row comes from
/// `UserRepository::get_by_email`.
pub(crate) const ADMIN_USER_EMAIL_SQL: &str =
    "SELECT email FROM users WHERE is_admin = TRUE ORDER BY created_at ASC LIMIT 1";

/// A user's tenant column, which is TEXT on both backends.
pub(crate) const USER_TENANT_SQL: &str = "SELECT tenant_id FROM users WHERE id = $1";

/// Every non-admin user id, oldest first.
pub(crate) const NON_ADMIN_USER_IDS_SQL: &str =
    "SELECT id FROM users WHERE is_admin = FALSE ORDER BY created_at";

/// How many non-admin users exist.
pub(crate) const COUNT_NON_ADMIN_USERS_SQL: &str =
    "SELECT COUNT(*) as cnt FROM users WHERE is_admin = FALSE";

/// Drop a tenant's LLM usage; `llm_usage.tenant_id` is text on both.
pub(crate) const DELETE_LLM_USAGE_BY_TENANT_SQL: &str =
    "DELETE FROM llm_usage WHERE tenant_id = $1";

/// One LLM usage row; the id, tenant and user columns are text on both.
pub(crate) const INSERT_LLM_USAGE_SQL: &str = "INSERT INTO llm_usage \
             (id, tenant_id, user_id, conversation_id, provider, model, \
              prompt_tokens, completion_tokens, total_tokens, call_type, \
              tool_calls_count, execution_time_ms, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)";

/// Drop a user's synthetic activities.
pub(crate) const DELETE_SYNTHETIC_BY_USER_SQL: &str =
    "DELETE FROM synthetic_activities WHERE user_id = $1";

/// One synthetic activity; the id and tenant columns are text on both.
pub(crate) const INSERT_SYNTHETIC_ACTIVITY_SQL: &str = "INSERT INTO synthetic_activities \
             (id, user_id, tenant_id, name, sport_type, start_date, duration_seconds, \
              distance_meters, elevation_gain, average_heart_rate, max_heart_rate, \
              average_speed, max_speed, calories, city, region, country, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)";

/// Insert or refresh a provider connection; every id column is text on both.
pub(crate) const UPSERT_PROVIDER_CONNECTION_SQL: &str = "INSERT INTO provider_connections \
             (id, user_id, tenant_id, provider, connection_type, connected_at, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT(user_id, tenant_id, provider) DO UPDATE SET \
               connection_type = EXCLUDED.connection_type, \
               connected_at = EXCLUDED.connected_at, \
               metadata = EXCLUDED.metadata";

/// A user's id by email.
pub(crate) const USER_ID_BY_EMAIL_SQL: &str = "SELECT id FROM users WHERE email = $1";

/// Insert or refresh a demo user. `$11` serves both `created_at` and
/// `last_active`; on conflict the credentials, role and locale are asserted
/// so a re-run restores the account to the seed's state.
pub(crate) const UPSERT_DEMO_USER_SQL: &str = "INSERT INTO users \
             (id, email, display_name, password_hash, tier, is_active, user_status, \
              is_admin, role, approved_at, created_at, last_active, auth_provider, locale) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, 'email', $12) \
             ON CONFLICT(email) DO UPDATE SET \
              password_hash = EXCLUDED.password_hash, \
              role = EXCLUDED.role, \
              is_admin = EXCLUDED.is_admin, \
              display_name = EXCLUDED.display_name, \
              locale = EXCLUDED.locale";

/// Insert or refresh a tenant; `ON CONFLICT(slug)` makes a re-run safe.
pub(crate) const UPSERT_TENANT_SQL: &str = "INSERT INTO tenants \
             (id, name, slug, subscription_tier, is_active, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, TRUE, $5, $6) \
             ON CONFLICT(slug) DO UPDATE SET \
              name = EXCLUDED.name, \
              subscription_tier = EXCLUDED.subscription_tier, \
              updated_at = EXCLUDED.updated_at";

/// Make a user the owner of a tenant, once; `$4` serves both timestamps.
pub(crate) const INSERT_TENANT_USER_SQL: &str = "INSERT INTO tenant_users \
             (id, tenant_id, user_id, role, invited_at, joined_at) \
             VALUES ($1, $2, $3, 'owner', $4, $4) \
             ON CONFLICT(tenant_id, user_id) DO NOTHING";

/// Point a user at a tenant; `users.tenant_id` is text on both.
pub(crate) const UPDATE_USER_TENANT_SQL: &str = "UPDATE users SET tenant_id = $1 WHERE id = $2";

/// An API key's id by name; `api_keys.id` is text on both.
pub(crate) const API_KEY_ID_BY_NAME_SQL: &str = "SELECT id FROM api_keys WHERE name = $1";

/// One API key with an hour-long window, active from the start.
pub(crate) const INSERT_API_KEY_SQL: &str = "INSERT INTO api_keys \
             (id, user_id, name, description, key_hash, key_prefix, tier, \
              rate_limit_requests, rate_limit_window_seconds, is_active, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 3600, TRUE, $9, $10)";

/// An A2A client's id by name; `a2a_clients.client_id` is text on both.
pub(crate) const A2A_CLIENT_ID_BY_NAME_SQL: &str =
    "SELECT client_id FROM a2a_clients WHERE name = $1";

/// One A2A client. The seed's `public_key` is the api-key hash and its
/// `client_secret` the secret hash, stored as given; `redirect_uris` takes
/// the column's empty-list default on both backends and the quotas are the
/// column defaults spelled out.
pub(crate) const INSERT_A2A_CLIENT_SQL: &str = "INSERT INTO a2a_clients \
             (client_id, user_id, name, description, api_key_hash, client_secret_hash, \
              capabilities, rate_limit_per_minute, rate_limit_per_day, \
              is_active, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1000, 10000, TRUE, $8, $9)";

/// One API-key usage row. `$id_col`/`$id_val` are the usage-id clause of
/// the backend: `", id"` / `", $6"` where the seed's uuid is the key, `""` /
/// `""` where the table mints its own.
macro_rules! insert_api_key_usage_sql {
    ($id_col:literal, $id_val:literal) => {
        concat!(
            "INSERT INTO api_key_usage \
             (api_key_id, timestamp, endpoint, status_code, response_time_ms",
            $id_col,
            ") VALUES ($1, $2, $3, $4, $5",
            $id_val,
            ")"
        )
    };
}
pub(crate) use insert_api_key_usage_sql;

/// One A2A usage row, on protocol `'1.0'`; the usage-id clause is as for
/// [`insert_api_key_usage_sql!`].
macro_rules! insert_a2a_usage_sql {
    ($id_col:literal, $id_val:literal) => {
        concat!(
            "INSERT INTO a2a_usage \
             (client_id, timestamp, endpoint, status_code, response_time_ms, protocol_version",
            $id_col,
            ") VALUES ($1, $2, $3, $4, $5, '1.0'",
            $id_val,
            ")"
        )
    };
}
pub(crate) use insert_a2a_usage_sql;

/// An agent's id and content hash by slug within a tenant.
pub(crate) const AGENT_BY_SLUG_SQL: &str =
    "SELECT id, content_hash FROM agents WHERE slug = $1 AND tenant_id = $2";

/// An agent's source and content hash by slug, tenant-agnostic on purpose:
/// used by `pierre-cli check-drift agents`, never by the seed path.
pub(crate) const AGENT_DRIFT_INFO_SQL: &str =
    "SELECT source, content_hash FROM agents WHERE slug = $1 LIMIT 1";

/// Every catalogue agent of a tenant, by slug.
pub(crate) const CATALOGUE_AGENTS_SQL: &str = concat!(
    "SELECT id, slug FROM agents \
     WHERE tenant_id = $1 AND is_system = TRUE AND slug IS NOT NULL \
       AND ",
    catalogue_source_filter!(),
    " ORDER BY slug"
);

/// Point a retired agent's conversations at no agent.
pub(crate) const DETACH_AGENT_CONVERSATIONS_SQL: &str =
    "UPDATE chat_conversations SET agent_id = NULL WHERE agent_id = $1";

/// Claim every legacy `'seed'` catalogue row of a tenant for contremaitre.
pub(crate) const TAKE_CATALOGUE_OWNERSHIP_SQL: &str = "UPDATE agents SET source = 'contremaitre' \
     WHERE tenant_id = $1 AND is_system = TRUE AND source = 'seed'";

/// Every distinct catalogue slug across tenants.
pub(crate) const CATALOGUE_SLUGS_SQL: &str = concat!(
    "SELECT DISTINCT slug FROM agents \
     WHERE is_system = TRUE AND slug IS NOT NULL \
       AND ",
    catalogue_source_filter!(),
    " ORDER BY slug"
);

/// One catalogue agent. `source = 'contremaitre'` flags the row for the
/// prompt-assembly registry overlay: `resolve_agent_base_prompt` reads the
/// live contremaitre prompt from `PromptRegistry` for that source and uses
/// `system_prompt` on a registry miss. The seeder is the single
/// contremaitre-only ingest path.
pub(crate) const INSERT_AGENT_SQL: &str = "INSERT INTO agents \
             (id, user_id, tenant_id, title, description, system_prompt, category, tags, \
              sample_prompts, token_count, created_at, updated_at, is_system, visibility, \
              slug, purpose, when_to_use, instructions, example_inputs, example_outputs, \
              success_criteria, prerequisites, source_file, content_hash, startup_query, \
              data_requirements, visuals, source) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, TRUE, $13, \
                     $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, 'contremaitre')";

/// Refresh a catalogue agent in place. `source = 'contremaitre'` is
/// re-stamped on every update so rows inserted before the column was
/// populated converge to the registry-overlay path on the next sync.
pub(crate) const UPDATE_AGENT_SQL: &str = "UPDATE agents SET \
               title = $1, description = $2, system_prompt = $3, category = $4, \
               tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8, \
               visibility = $9, purpose = $10, when_to_use = $11, instructions = $12, \
               example_inputs = $13, example_outputs = $14, success_criteria = $15, \
               prerequisites = $16, source_file = $17, content_hash = $18, startup_query = $19, \
               data_requirements = $20, visuals = $21, source = 'contremaitre' \
             WHERE id = $22";

/// One agent relation, once; the UNIQUE triple makes a re-run a no-op.
pub(crate) const INSERT_AGENT_RELATION_SQL: &str = "INSERT INTO agent_relations \
             (id, agent_id, related_agent_id, relation_type, created_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT DO NOTHING";

/// The author row a user already holds in a tenant, if any.
pub(crate) const AGENT_AUTHOR_ID_SQL: &str =
    "SELECT id FROM agent_authors WHERE user_id = $1 AND tenant_id = $2";

/// One author row, unverified with zero counters.
pub(crate) const INSERT_AGENT_AUTHOR_SQL: &str = "INSERT INTO agent_authors \
             (id, user_id, tenant_id, display_name, is_verified, \
              published_agent_count, total_install_count, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, FALSE, 0, 0, $5, $6)";

/// One published store listing, once; `$4` is the listing's creation
/// instant and serves `published_at`, `created_at` and `updated_at`.
pub(crate) const INSERT_STORE_LISTING_SQL: &str = "INSERT INTO store_listings \
             (id, agent_id, tenant_id, publish_status, published_at, install_count, \
              author_id, created_at, updated_at) \
             VALUES ($1, $2, $3, 'published', $4, 0, $5, $4, $4) \
             ON CONFLICT DO NOTHING";

/// Insert or refresh an agent's translation, keyed on `(agent_id, locale)`;
/// a re-run after a file edit refreshes the content and its sha.
pub(crate) const UPSERT_AGENT_TRANSLATION_SQL: &str = "INSERT INTO agent_translations \
             (agent_id, locale, title, description, purpose, instructions, source_sha, tags, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP) \
             ON CONFLICT (agent_id, locale) DO UPDATE SET \
               title = EXCLUDED.title, \
               description = EXCLUDED.description, \
               purpose = EXCLUDED.purpose, \
               instructions = EXCLUDED.instructions, \
               source_sha = EXCLUDED.source_sha, \
               tags = EXCLUDED.tags, \
               updated_at = CURRENT_TIMESTAMP";
