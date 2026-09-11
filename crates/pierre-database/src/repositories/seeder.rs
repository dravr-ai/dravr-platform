// ABOUTME: Repository trait definitions for the seed-only repository operations domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
/// Every catalogue query spells the same pair, so it is spelled once — the
/// engine-specific halves (`is_system = 1` vs `TRUE`, the `::uuid` cast) stay
/// inline where they differ.
pub const CATALOGUE_SOURCE_FILTER: &str = "source IN ('contremaitre', 'seed')";

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
/// These three predate the id-keyed pointers and none of them carries a
/// foreign key, so a retired slug leaves them pointing at a name the
/// catalogue no longer knows: the athlete's learned playbooks, the advice
/// waiting to be delivered, and the training plan that was built for them.
/// `$1` is the successor's slug, `$2` the retired one.
pub const AGENT_SLUG_REWRITES: [&str; 3] = [
    "UPDATE coaching_playbooks SET agent_slug = $1 WHERE agent_slug = $2",
    "UPDATE pending_advice SET agent_slug = $1 WHERE agent_slug = $2",
    "UPDATE training_plans SET agent_slug = $1 WHERE agent_slug = $2",
];
