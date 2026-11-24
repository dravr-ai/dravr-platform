// ABOUTME: Core database management with migration system for SQLite and PostgreSQL
// ABOUTME: Handles schema setup, user management, API keys, analytics, and A2A authentication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Agent-to-Agent (A2A) authentication and usage tracking
pub mod a2a;
/// The A2A reaper's bulk fail of tasks a dead instance left non-terminal
pub mod a2a_task_reaper;
/// Account deletion with its receipt (`SQLite`) backing `AccountDeletionRepository`.
pub mod account_deletion;
/// Whole-account reset (`SQLite`) backing `AccountResetRepository`.
pub mod account_reset;
/// Historical activity backfills still owed, one per (user, provider), leased to one re-runner
pub mod activity_backfill_jobs;
/// Provider-agnostic activity cache (stale-while-revalidate) persistence
pub mod activity_cache_persistence;
/// The thread each activity's view opened, one per activity (`SQLite`)
pub mod activity_conversations;
/// One stored route read per activity: a trimmed track or the reason there is none (`SQLite`)
pub mod activity_route_tracks;
/// The one per-activity sample store: pushed, uploaded or imported samples (`SQLite`)
pub mod activity_streams;
/// Admin token management and authorization
pub mod admin;
/// The admin audit trail (`SQLite`) backing `AdminAuditRepository`.
pub mod admin_audit;
/// Agent package artefacts — flavour, skeleton, workouts stored per agent (`SQLite`)
pub mod agent_artefacts;
/// Parked agent drafts (`SQLite`) backing `AgentDraftsRepository`.
pub mod agent_drafts;
/// Coaches (custom AI personas) storage and management
pub mod agents;
/// Analytics and usage statistics database operations
pub mod analytics;
/// API key management and validation
pub mod api_keys;
/// iOS App Attest keys of the app installs Apple attested (`SQLite`)
pub mod app_attest_keys;
/// Archive erasure outbox (`SQLite`) backing `ArchiveErasureRepository`.
pub mod archive_erasures;
/// The athlete's gear ledger (`SQLite`) backing `AthleteGearRepository`.
pub mod athlete_gear;
/// The places an athlete trains from (`SQLite`) backing `AthletePlaceRepository`.
pub mod athlete_places;
/// The calendar an athlete chose for Dravr's pushes (`SQLite`)
pub mod calendar_preferences;
/// Chat conversation and message storage
pub mod chat;
/// The check-in cadence each athlete chose (`SQLite`)
pub mod check_in_preferences;
/// Claim verdicts from the bullshit detector pipeline
pub mod claim_verdicts;
/// Coach-access requests an admin decides (`SQLite`)
pub mod coach_access_requests;
/// A coach's "Traité" on Coach Home attention reasons (`SQLite`)
pub mod coach_attention_dismissals;
/// The athlete's record of their human coach's reads (`SQLite`)
pub mod coach_data_reads;
/// A coach's review of the agent's group replies (`SQLite`)
pub mod coach_reply_reviews;
/// Coaching group storage, membership, and invite management
pub mod coaching_groups;
/// Athlete commitments (`SQLite`) backing `CommitmentRepository`.
pub mod commitments;
/// The athlete's statutory data access requests (`SQLite`)
pub mod data_access_requests;
/// Delegated connections (`SQLite`) backing `DelegatedConnectionRepository`.
pub mod delegated_connections;
/// Email-verification token management — proving a registered address
pub mod email_verification_tokens;
/// The admin Engagement screen's scoped reads (`SQLite`) backing `EngagementRepository`.
pub mod engagement;
/// Database error types
pub mod errors;
/// Tenant defaults + per-user overrides (`SQLite`) backing `FeatureFlagsRepository`.
pub mod feature_flags;
/// The external sign-in identities (a Google account id) each account is reached by (`SQLite`)
pub mod federated_identities;
/// The in-app feedback thread with the Dravr team (`SQLite`)
pub mod feedback_messages;
/// Firebase identity deletion outbox (`SQLite`) backing `FirebaseIdentityDeletionRepository`.
pub mod firebase_identity_deletions;
/// The days each athlete met a first-run flow's done rule (`SQLite`)
pub mod first_run_progress;
/// User fitness configuration storage and retrieval
pub mod fitness_configurations;
/// Every batch Garmin pushes, written before the 200 and worked under a lease (`SQLite`)
pub mod garmin_push_inbox;
/// Health persistence: data sources, sleep, recovery, health snapshots
pub mod health_persistence;
/// Health records: self-reports, clinical records and lab documents (carnet#1544).
pub mod health_records;
/// Impersonation session management for admin user impersonation
pub mod impersonation;
/// Import tombstones (`SQLite`) backing `ImportTombstoneRepository`.
pub mod import_tombstones;
/// LLM credential management and admin config overrides
pub mod llm_credentials;
/// LLM usage tracking for cost analysis and quota enforcement
pub mod llm_usage;
/// MCP Tasks extension handle persistence
pub mod mcp_tasks;
/// Coaching harness memory foundations (facts, compaction, sessions)
pub mod memory;
/// Post-turn memory extractions still owed, leased to one re-runner at a time
pub mod memory_extraction_jobs;
/// Multi-channel messaging gateway (channel configs, sessions, messages, queue)
pub mod messaging;
pub mod messaging_link_states;
/// Reaction → chat-message resolution for the shared per-message feedback write
pub mod messaging_reactions;
/// The embedded `SQLite` migration set
mod migrations;
/// Mobility features (stretching exercises and yoga poses)
pub mod mobility;
/// OAuth client-state persistence (CSRF `state` + PKCE verifier)
mod oauth_client_state;
/// OAuth callback notification handling
pub mod oauth_notifications;
/// Password reset token management for admin-initiated password resets
pub mod password_reset_tokens;
/// A member's record of their peers' reads (`SQLite`)
pub mod peer_data_reads;
/// The persona-held notifications each digest returned (`SQLite`)
pub mod persona_digest_returns;
/// Personal bests at the standard running distances and the runs scanned for them (`SQLite`)
pub mod personal_bests;
/// The once-claims on the coach's own posts to an athlete (`SQLite`)
pub mod platform_post_claims;
/// `SQLite` `PlaybookRepository` impl — procedural coaching memory.
pub mod playbooks;
/// Pre-approved email allow-list consulted at registration (`SQLite`)
pub mod pre_approved_emails;
/// Endurance `prescribed_workouts` audit-trail repository (`SQLite`)
pub mod prescribed_workouts;
/// Provider connections: unified connection tracking for all provider types
pub mod provider_connections;
/// Provider data purge: disconnect and termination deletes (`SQLite`)
pub mod provider_data;
/// Newest-receipt-wins writes for data a provider pushes (`SQLite`)
pub mod pushed_data;
/// Recipe storage and management for nutrition planning
pub mod recipes;
/// Repository trait definitions for focused database access
pub mod repositories;
/// Messaging turns the shutdown drain handed off, leased to one re-runner at a time
pub mod resumable_turns;
/// Endurance cached GPX `route_summaries` repository (`SQLite`)
pub mod route_summaries;
/// Routines and when each fires next (`SQLite`)
pub mod routines;
/// Security repository: the RSA signing keypair and the system secrets
pub mod security_repository;
/// Seeder repository for seed-only database operations
pub mod seeder;
/// First-party refresh tokens — the credential a device holds between JWTs
pub mod session_refresh_tokens;
/// URL shortener: `code` → `target_url` with an integer-epoch TTL (`SQLite`)
pub mod short_links;
/// Admin signup codes and their redemptions (`SQLite`)
pub mod signup_codes;
/// The new-user cohort the admin console and the daily digest read (`SQLite`)
pub mod signup_cohort;
/// Store listings for agent publishing workflow (`SQLite`)
pub mod store_listings;
/// Warnings sent before a Strava seat is reclaimed (`SQLite`)
pub mod strava_seat_reclaim_warnings;
/// Stripe-backed subscription persistence (Phase 5 billing)
pub mod subscriptions;
/// System settings for admin-configurable options
pub mod system_settings;
/// Tenant management: CRUD, OAuth credentials, user-tenant roles
pub mod tenants;
/// `OAuth2` server token management (clients, auth codes, refresh tokens, state)
pub mod tokens;
/// Tool selection and per-tenant MCP tool configuration
pub mod tool_selection;
/// Endurance daily `training_history` rollup repository (`SQLite`)
pub mod training_history;
/// `SQLite` training-plan persistence.
pub mod training_plans;
/// The `.fit` files athletes uploaded, one row per file (`SQLite`)
pub mod uploaded_activity_files;
/// Usage counters for rate limiting and quota enforcement
pub mod usage_counters;
/// Use-case starters shown to and tapped by each athlete (`SQLite`)
pub mod use_case_exposures;
/// An athlete's own data, one page at a time, for the portable export (`SQLite`)
pub mod user_export;
/// The SQLite originals-bundle state, emitted from the shared implementation.
pub mod user_export_bundles;
/// Per-user Home preferences (`SQLite`) backing `HomePreferencesRepository`.
pub mod user_home_preferences;
/// Single-use user invitation links (`SQLite`)
pub mod user_invitations;
/// User MCP token management for AI client authentication
pub mod user_mcp_tokens;
/// User OAuth token storage and management
pub mod user_oauth_tokens;
/// Durable per-user onboarding step completion state (`SQLite`)
pub mod user_onboarding;
/// Endurance typed `UserPhysiologicalProfile` + `Dossier` composer (`SQLite`)
pub mod user_physiological_profiles;
/// Single-column preference writes on the users row (locale, persona, theme, …)
pub mod user_preferences;
/// Per-user rate-limit exemption table (`SQLite`) backing `UserRateLimitOverrideRepository`.
pub mod user_rate_limit_overrides;
/// Per-user admin tier override table (`SQLite`) backing `UserTierOverrideRepository`.
pub mod user_tier_overrides;
/// Per-user admin tool override table (`SQLite`) backing `UserToolOverrideRepository`.
pub mod user_tool_overrides;
/// Per-user unit preference inputs (`SQLite`) backing `UnitPreferencesRepository`.
pub mod user_unit_preferences;
/// User account management and authentication
pub mod users;
/// dravr-meteo persistent weather cache (geographic + hourly buckets)
pub mod weather_cache;
/// dravr.ai docs sign-in link tokens (`SQLite`)
pub mod website_sign_in_tokens;
/// The periodic-worker ledger, claimed in one statement so two instances tick once
pub mod worker_runs;
/// Endurance user-authored `workout_templates` repository (`SQLite`)
pub mod workout_templates;

pub use agents::{
    Agent, AgentCategory, CreateAgentRequest, ListAgentsFilter, PublishStatus, UpdateAgentRequest,
};
// The chat DTOs are canonical in pierre-core; re-exported here for the crates
// that reach them by this path.
pub use errors::{DatabaseError, DatabaseResult};
pub use oauth_notifications::OAuthNotification;
pub use pierre_core::models::agents::{AgentWithListing, StoreListing};
pub use pierre_core::models::{
    AddMessageParams, ConversationPage, ConversationParticipant, ConversationRecord,
    ConversationSummary, MessageFeedbackRecord, MessageRecord, UpsertMessageFeedbackParams,
};
pub use pierre_core::models::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};

use crate::backends::shared::encryption::{
    hmac_token_for_storage, open_with_aad, seal_with_aad, DekKeys,
};
use crate::backends::{shared, DatabaseProvider};
use crate::database::migrations::SQLITE_MIGRATIONS;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::{Pool, Sqlite, SqlitePool};
use std::collections::HashMap;
use std::fmt;
use tracing::info;

/// Database connection pool with encryption support
#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
    /// Active DEK used to encrypt new data; its version is `active_dek_version`.
    encryption_key: Vec<u8>,
    /// Version of the active DEK held in `encryption_key`. New ciphertext is
    /// tagged with this version; legacy un-prefixed ciphertext is version 1.
    active_dek_version: u32,
    /// Retired DEK versions retained for decrypting older ciphertext (version -> key).
    prior_dek_versions: HashMap<u32, Vec<u8>>,
    /// Stable key for the refresh-token blind index (HMAC). Pinned to DEK v1 so
    /// deterministic lookups survive DEK rotation — stored hashes cannot be
    /// recomputed (the plaintext token is never retained).
    blind_index_key: Vec<u8>,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("pool", &self.pool)
            .field("encryption_key", &"[REDACTED]")
            .field("active_dek_version", &self.active_dek_version)
            .field("prior_dek_versions", &"[REDACTED]")
            .field("blind_index_key", &"[REDACTED]")
            .finish()
    }
}

impl Database {
    /// Create a new database connection (internal implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database URL is invalid or malformed
    /// - Database connection fails
    /// - `SQLite` file creation fails
    /// - Migration process fails
    /// - Encryption key is invalid
    async fn new_impl(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        let connection_options = shared::connection_url::sqlite_connection_options(database_url);

        let pool = SqlitePool::connect(&connection_options)
            .await
            .map_err(|e| AppError::database(format!("Failed to connect to database: {e}")))?;

        Self::from_pool(pool, encryption_key).await
    }

    /// Open the database over a pool the caller has already configured.
    ///
    /// For a caller that needs connection options a URL cannot express — a
    /// journal mode, a synchronous level, a single connection. The embedded
    /// migrations run against the pool, so a database that already carries
    /// them applies nothing and an empty one ends up fully migrated.
    ///
    /// # Errors
    ///
    /// Returns an error if the migration process fails.
    pub async fn from_pool(pool: Pool<Sqlite>, encryption_key: Vec<u8>) -> AppResult<Self> {
        let db = Self {
            pool,
            blind_index_key: encryption_key.clone(),
            active_dek_version: shared::encryption::LEGACY_DEK_VERSION,
            prior_dek_versions: HashMap::new(),
            encryption_key,
        };

        db.migrate_impl()
            .await
            .map_err(|e| AppError::database(format!("Database migration failed: {e}")))?;

        Ok(db)
    }

    /// Create a new database connection (public API)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database URL is invalid or malformed
    /// - Database connection fails
    /// - `SQLite` file creation fails
    /// - Migration process fails
    /// - Encryption key is invalid
    pub async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        Self::new_impl(database_url, encryption_key).await
    }

    /// Get a reference to the database pool for advanced operations
    #[must_use]
    pub const fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    /// Update the encryption key used for token encryption/decryption
    ///
    /// This is called after the actual DEK is loaded from the database during
    /// two-tier key management initialization. The database is initially created
    /// with a temporary key, then updated with the real key once it's loaded.
    ///
    /// # Safety
    /// Only call this once during startup, before any encrypted data operations.
    pub fn update_encryption_key(&mut self, new_key: Vec<u8>) {
        // At bootstrap the loaded key is DEK v1, which also anchors the blind index.
        self.blind_index_key.clone_from(&new_key);
        self.encryption_key = new_key;
    }

    /// Install a full set of DEK versions (after load-all-versions or a rotation).
    ///
    /// `active_key` (version `active_version`) encrypts new data; `prior_versions`
    /// are retained for decrypt-only. The blind-index (HMAC) key is pinned to
    /// version 1, falling back to the active key only when v1 is itself active.
    ///
    /// # Safety
    /// Call during startup or rotation while holding `&mut self`, before serving
    /// concurrent encrypted-data operations.
    pub fn install_dek_versions(
        &mut self,
        active_version: u32,
        active_key: Vec<u8>,
        prior_versions: HashMap<u32, Vec<u8>>,
    ) {
        self.blind_index_key = prior_versions
            .get(&shared::encryption::LEGACY_DEK_VERSION)
            .cloned()
            .unwrap_or_else(|| active_key.clone());
        self.active_dek_version = active_version;
        self.prior_dek_versions = prior_versions;
        self.encryption_key = active_key;
    }

    /// The DEKs this database holds, borrowed for one envelope operation:
    /// the active DEK for new ciphertext, the retained prior ones to decrypt.
    fn dek_keys(&self) -> DekKeys<'_> {
        DekKeys {
            active_version: self.active_dek_version,
            active_key: &self.encryption_key,
            prior_versions: &self.prior_dek_versions,
        }
    }

    /// Run all database migrations (public API)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any migration fails
    /// - Database connection is lost during migration
    /// - Insufficient database permissions
    pub async fn migrate(&self) -> AppResult<()> {
        self.migrate_impl().await
    }

    /// Encrypt data using AES-256-GCM with AAD (public API)
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails
    pub fn encrypt_data_with_aad(&self, data: &str, aad_context: &str) -> AppResult<String> {
        seal_with_aad(&self.dek_keys(), data, aad_context)
    }

    /// Decrypt data using AES-256-GCM with AAD (public API)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Decryption fails
    /// - Data is malformed
    /// - AAD context does not match
    pub fn decrypt_data_with_aad(
        &self,
        encrypted_data: &str,
        aad_context: &str,
    ) -> AppResult<String> {
        open_with_aad(&self.dek_keys(), encrypted_data, aad_context)
    }

    /// Run all database migrations (internal implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any migration fails
    /// - Database connection is lost during migration
    /// - Insufficient database permissions
    async fn migrate_impl(&self) -> AppResult<()> {
        info!("Running database migrations...");

        SQLITE_MIGRATIONS
            .run(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Migration failed: {e}")))?;

        info!("Database migrations completed successfully");
        Ok(())
    }

    /// Get user role for a specific tenant
    ///
    /// # Errors
    ///
    /// Returns an error if database query fails
    pub async fn get_user_role(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT role FROM tenant_users WHERE user_id = ? AND tenant_id = ?",
        )
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database query failed: {e}")))?;

        Ok(row.map(|r| r.0))
    }
}

// Implement HasEncryption trait for SQLite over the shared AES-256-GCM envelope
impl shared::encryption::HasEncryption for Database {
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        seal_with_aad(&self.dek_keys(), data, aad)
    }

    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        open_with_aad(&self.dek_keys(), encrypted, aad)
    }

    fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
        // Pinned to the blind-index key (DEK v1) so lookups survive DEK rotation.
        Ok(hmac_token_for_storage(&self.blind_index_key, token))
    }
}

use async_trait::async_trait;

#[async_trait]
impl DatabaseProvider for Database {
    async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        // Call inherent impl directly to avoid infinite recursion
        Self::new_impl(database_url, encryption_key).await
    }
    async fn migrate(&self) -> AppResult<()> {
        // Call inherent impl directly
        Self::migrate_impl(self).await
    }
}

/// Generate a secure encryption key (32 bytes for AES-256)
#[must_use]
pub fn generate_encryption_key() -> [u8; 32] {
    use rand::Rng;
    let mut key = [0u8; 32];
    rand::rng().fill(&mut key);
    key
}
