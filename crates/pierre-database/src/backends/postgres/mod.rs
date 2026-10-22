// ABOUTME: PostgreSQL database implementation for cloud and production deployments
// ABOUTME: Provides enterprise-grade database support with connection pooling and scalability
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! `PostgreSQL` database implementation
//!
//! This module provides `PostgreSQL` support for cloud deployments,
//! implementing the same interface as the `SQLite` version.

/// A2A protocol repository implementations
pub mod a2a;
/// The A2A reaper's bulk fail of tasks a dead instance left non-terminal
pub mod a2a_task_reaper;
/// Account deletion with its receipt (`Postgres`) backing `AccountDeletionRepository`.
pub mod account_deletion;
/// Historical activity backfills still owed, one per (user, provider), leased to one re-runner
pub mod activity_backfill_jobs;
/// Provider-agnostic activity cache (stale-while-revalidate) repository implementation
pub mod activity_cache_persistence;
/// The thread each activity's view opened, one per activity (Postgres)
pub mod activity_conversations;
/// One stored route read per activity: a trimmed track or the reason there is none (Postgres)
pub mod activity_route_tracks;
/// Admin, impersonation, and MCP token repository implementations
pub mod admin;
/// Agent package artefacts — flavour, skeleton, workouts stored per agent (Postgres)
pub mod agent_artefacts;
/// Coaches repository implementation
pub mod agents;
/// Usage tracking repository implementation: API-key and JWT usage, request logs, top tools
pub mod analytics;
/// API key repository implementation
pub mod api_key;
/// iOS App Attest keys of the app installs Apple attested (Postgres)
pub mod app_attest_keys;
/// The athlete's gear ledger (`Postgres`) backing `AthleteGearRepository`.
pub mod athlete_gear;
/// Chat repository implementation
pub mod chat;
/// The check-in cadence each athlete chose (`PostgreSQL`)
pub mod check_in_preferences;
/// Claim verdict repository implementation
pub mod claim_verdicts;
/// Coach-access requests a super-admin decides (Postgres)
pub mod coach_access_requests;
/// A coach's review of the agent's group replies (Postgres)
pub mod coach_reply_reviews;
/// Coaching group repository implementation (group CRUD, membership, invites)
pub mod coaching_groups;
/// Athlete commitments (`Postgres`) backing `CommitmentRepository`.
pub mod commitments;
/// Delegated connections (`Postgres`) backing `DelegatedConnectionRepository`.
pub mod delegated_connections;
/// Email-verification token management — proving a registered address
pub mod email_verification;
/// Encryption support (AES-256-GCM)
pub mod encryption;
/// Tenant defaults + per-user overrides (`Postgres`) backing `FeatureFlagsRepository`.
pub mod feature_flags;
/// The external sign-in identities (a Google account id) each account is reached by (Postgres)
pub mod federated_identities;
/// Firebase identity deletion outbox (`Postgres`) backing `FirebaseIdentityDeletionRepository`.
pub mod firebase_identity_deletions;
/// The days each athlete met a first-run flow's done rule (`PostgreSQL`)
pub mod first_run_progress;
/// Fitness configuration — tenant- and user-scoped training settings
pub mod fitness_config;
/// Guardian pending actions (`Postgres`) backing `GuardianPendingActionsRepository`.
pub mod guardian_actions;
/// Health persistence: data sources, sleep, recovery, health snapshots
pub mod health_persistence;
/// Super-admin impersonation session audit records.
pub mod impersonation;
/// Import tombstones (`Postgres`) backing `ImportTombstoneRepository`.
pub mod import_tombstones;
/// LLM credential repository implementation
pub mod llm_credentials;
/// LLM usage tracking repository implementation
pub mod llm_usage;
/// MCP Tasks extension handle repository implementation
pub mod mcp_tasks;
/// Coaching harness memory (compaction, facts, notes, followups, sessions)
pub mod memory;
/// Post-turn memory extractions still owed, leased to one re-runner at a time
pub mod memory_extraction_jobs;
/// Messaging gateway repository implementations
pub mod messaging;
pub mod messaging_link_states;
/// Reaction → chat-message resolution for the shared per-message feedback write
pub mod messaging_reactions;
/// Mobility repository implementation (stretching exercises and yoga poses)
pub mod mobility;
/// OAuth client-state repository implementation (CSRF `state` + PKCE verifier)
mod oauth_client_state;
/// OAuth completion notification repository implementation
pub mod oauth_notifications;
/// One-time password reset tokens issued by admins and the self-service flow
pub mod password_reset_tokens;
/// The persona-held notifications each digest returned (`PostgreSQL`)
pub mod persona_digest_returns;
/// Personal bests at the standard running distances and the runs scanned for them (`PostgreSQL`)
pub mod personal_bests;
/// The once-claims on the coach's own posts to an athlete (`PostgreSQL`)
pub mod platform_post_claims;
/// Postgres `PlaybookRepository` impl — procedural coaching memory.
pub mod playbooks;
/// Pre-approved email allow-list consulted at registration (Postgres)
pub mod pre_approved_emails;
/// Endurance `prescribed_workouts` audit-trail repository (Postgres)
pub mod prescribed_workouts;
/// Provider connections — the single source of truth for provider connectivity
pub mod provider_connections;
/// Provider data purge: disconnect and termination deletes (Postgres)
pub mod provider_data;
/// Recipe repository implementation (CRUD with nutrition caching)
pub mod recipes;
/// Messaging turns the shutdown drain handed off, leased to one re-runner at a time
pub mod resumable_turns;
/// Endurance cached GPX `route_summaries` repository (Postgres)
pub mod route_summaries;
/// Security and notification repository implementations
pub mod security;
/// Seeder repository for seed-only database operations
pub mod seeder;
/// First-party refresh tokens — the credential a device holds between JWTs (Postgres)
pub mod session_refresh_tokens;
/// URL shortener: `code` → `target_url` with an integer-epoch TTL (`PostgreSQL`)
pub mod short_links;
/// Store listings repository implementation (marketplace publishing workflow)
pub mod store_listings;
/// Warnings sent before a Strava seat is reclaimed (`PostgreSQL`)
pub mod strava_seat_reclaim_warnings;
/// Stripe-backed subscription persistence (Phase 5 billing)
pub mod subscriptions;
/// Tenant repository implementation
pub mod tenant;
/// OAuth 2.0 server persistence — clients, codes, refresh tokens, states, grants, device codes
pub mod tokens;
/// Tool catalog and per-tenant tool override repository implementation
pub mod tool_selection;
/// Endurance daily `training_history` rollup repository (Postgres)
pub mod training_history;
/// `PostgreSQL` training-plan persistence.
pub mod training_plans;
/// The `.fit` files athletes uploaded, one row per file (Postgres)
pub mod uploaded_activity_files;
/// Usage counter repository implementation
pub mod usage_counters;
/// Use-case starters shown to and tapped by each athlete (`PostgreSQL`)
pub mod use_case_exposures;
/// User and profile repository implementations
pub mod user;
/// Per-user Home preferences (`Postgres`) backing `HomePreferencesRepository`.
pub mod user_home_preferences;
/// Single-use user invitation links (Postgres)
pub mod user_invitations;
/// User MCP tokens for AI client authentication (Postgres)
pub mod user_mcp_tokens;
/// Per-user, per-tenant provider OAuth tokens, Strava pool apps and BYO OAuth apps
pub mod user_oauth_tokens;
/// Durable per-user onboarding step completion state (Postgres)
pub mod user_onboarding;
/// Endurance typed `UserPhysiologicalProfile` + `Dossier` composer (Postgres)
pub mod user_physiological_profiles;
/// Single-column preference writes on the users row (locale, persona, theme, …)
pub mod user_preferences;
/// Per-user rate-limit exemption table (`Postgres`) backing `UserRateLimitOverrideRepository`.
pub mod user_rate_limit_overrides;
/// Per-user admin tier override table (`Postgres`) backing `UserTierOverrideRepository`.
pub mod user_tier_overrides;
/// Per-user admin tool override table (`Postgres`) backing `UserToolOverrideRepository`.
pub mod user_tool_overrides;
/// Per-user unit preference inputs (`Postgres`) backing `UnitPreferencesRepository`.
pub mod user_unit_preferences;
/// dravr-meteo persistent weather cache (geographic + hourly buckets)
pub mod weather_cache;
/// dravr.ai docs sign-in link tokens (Postgres)
pub mod website_sign_in_tokens;
/// The periodic-worker ledger, claimed in one statement so two instances tick once
pub mod worker_runs;
/// Endurance user-authored `workout_templates` repository (Postgres)
pub mod workout_templates;

use super::{shared, DatabaseProvider};
use crate::database::system_settings::{SystemSetting, SETTING_AUTO_APPROVAL_ENABLED};
use crate::repositories::system_settings::{
    impl_system_settings, system_setting_from_row, GET_SYSTEM_SETTING_SQL, SET_SYSTEM_SETTING_SQL,
};
use async_trait::async_trait;
use chrono::Utc;
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Pool, Postgres};
use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{info, warn};

/// The `PostgreSQL` migration set compiled into this binary.
///
/// Embedded once so that both [`migrate`](DatabaseProvider::migrate) and
/// [`migrations_fingerprint`] read the same set: the fingerprint names the
/// test template database that already carries exactly these migrations, and a
/// template built from a different set must never be mistaken for it.
static PG_MIGRATIONS: Migrator = sqlx::migrate!("./migrations_pg");

/// The embedded migrator configured the way production applies it.
///
/// `ignore_missing` is on so a database carrying a migration this binary no
/// longer knows (a rollback deploy) still starts; `Migrator` cannot be mutated
/// in a `static`, so the runtime copy borrows the embedded set instead.
fn pg_migrator() -> Migrator {
    Migrator {
        migrations: Cow::Borrowed(PG_MIGRATIONS.migrations.as_ref()),
        ignore_missing: true,
        locking: PG_MIGRATIONS.locking,
        no_tx: PG_MIGRATIONS.no_tx,
    }
}

/// A short, stable digest of the embedded `PostgreSQL` migration set.
///
/// Covers every migration's version and checksum in order, so any edit,
/// addition, or removal yields a different value. Test isolation uses it to
/// name the template database that has these exact migrations applied.
#[must_use]
pub fn migrations_fingerprint() -> String {
    let mut hasher = Sha256::new();
    for migration in PG_MIGRATIONS.iter() {
        hasher.update(migration.version.to_le_bytes());
        hasher.update(&migration.checksum);
    }
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

/// `PostgreSQL` database implementation
#[derive(Clone)]
pub struct PostgresDatabase {
    pool: Pool<Postgres>,
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
    /// Until when boot-time key-store reads keep queueing for a free
    /// connection (`PostgresPoolConfig::boot_wait_secs` after the pool opened)
    boot_deadline: Instant,
}

impl PostgresDatabase {
    /// Until when boot-time key-store reads keep queueing for a free
    /// connection; see
    /// [`KeyStoreQueue`](crate::backends::shared::key_material::KeyStoreQueue)
    #[must_use]
    pub const fn boot_deadline(&self) -> Instant {
        self.boot_deadline
    }

    /// Get a reference to the `PostgreSQL` connection pool
    #[must_use]
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Close the database connection pool
    pub async fn close(&self) {
        self.pool.close().await;
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

    /// Resolve the DEK bytes for a given ciphertext version.
    ///
    /// Returns the active DEK for the active version, a retained prior DEK for an
    /// older version, or an error if no key for that version is held.
    ///
    /// # Errors
    ///
    /// Returns an error if no DEK is available for `version`.
    fn dek_key_for_version(&self, version: u32) -> AppResult<&[u8]> {
        if version == self.active_dek_version {
            Ok(&self.encryption_key)
        } else if let Some(key) = self.prior_dek_versions.get(&version) {
            Ok(key)
        } else {
            Err(AppError::database(format!(
                "No DEK available for ciphertext version {version} (active version is {})",
                self.active_dek_version
            )))
        }
    }
}

impl PostgresDatabase {
    /// Create new `PostgreSQL` database with provided pool configuration (internal implementation)
    /// This is called by the Database factory with centralized `ServerConfig`
    ///
    /// # Errors
    ///
    /// Returns an error if database connection or pool configuration fails
    async fn new_impl(
        database_url: &str,
        encryption_key: Vec<u8>,
        pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        info!(
            max_connections = pool_config.max_connections,
            min_connections = pool_config.min_connections,
            acquire_timeout_secs = pool_config.acquire_timeout_secs,
            idle_timeout_secs = pool_config.idle_timeout_secs,
            max_lifetime_secs = pool_config.max_lifetime_secs,
            boot_wait_secs = pool_config.boot_wait_secs,
            retries = pool_config.connection_retries,
            "PostgreSQL pool config"
        );

        let pool = Self::connect_with_retry(database_url, pool_config).await?;
        let boot_deadline = Instant::now() + Duration::from_secs(pool_config.boot_wait_secs);

        let db = Self {
            pool,
            blind_index_key: encryption_key.clone(),
            active_dek_version: shared::encryption::LEGACY_DEK_VERSION,
            prior_dek_versions: HashMap::new(),
            encryption_key,
            boot_deadline,
        };

        // Run migrations
        db.migrate().await?;

        Ok(db)
    }

    /// Connect to `PostgreSQL` with exponential backoff retry on failure
    ///
    /// Handles transient connection failures (network issues, database restarts)
    /// by retrying with increasing delays between attempts.
    async fn connect_with_retry(
        database_url: &str,
        pool_config: &PostgresPoolConfig,
    ) -> AppResult<Pool<Postgres>> {
        let connect_options = PgConnectOptions::from_str(database_url)
            .map_err(|e| AppError::config(format!("Invalid PostgreSQL URL: {e}")))?
            .statement_cache_capacity(pool_config.statement_cache_capacity);
        let pool_options = PgPoolOptions::new()
            .max_connections(pool_config.max_connections)
            .min_connections(pool_config.min_connections)
            .acquire_timeout(Duration::from_secs(pool_config.acquire_timeout_secs))
            .idle_timeout(Some(Duration::from_secs(pool_config.idle_timeout_secs)))
            .max_lifetime(Some(Duration::from_secs(pool_config.max_lifetime_secs)))
            .test_before_acquire(pool_config.test_before_acquire);
        let max_retries = pool_config.connection_retries;

        let mut last_error = None;
        let mut delay_ms = pool_config.initial_retry_delay_ms;

        for attempt in 0..=max_retries {
            match pool_options
                .clone()
                .connect_with(connect_options.clone())
                .await
            {
                Ok(pool) => {
                    if attempt > 0 {
                        info!(
                            "PostgreSQL connection established after {} retries",
                            attempt
                        );
                    }
                    return Ok(pool);
                }
                Err(e) => {
                    last_error = Some(e);

                    if attempt < max_retries {
                        warn!(
                            "PostgreSQL connection attempt {}/{} failed, retrying in {}ms: {}",
                            attempt + 1,
                            max_retries + 1,
                            delay_ms,
                            last_error.as_ref().map_or("unknown", |e| e
                                .as_database_error()
                                .map_or("connection error", |de| de.message()))
                        );
                        sleep(Duration::from_millis(delay_ms)).await;
                        // Exponential backoff with cap
                        delay_ms = (delay_ms * 2).min(pool_config.max_retry_delay_ms);
                    }
                }
            }
        }

        // All retries exhausted
        Err(AppError::database(format!(
            "Failed to connect to PostgreSQL after {} retries: {}",
            max_retries + 1,
            last_error.map_or_else(|| "unknown error".to_owned(), |e| e.to_string())
        )))
    }

    /// Create new `PostgreSQL` database with provided pool configuration (public API)
    /// This is called by the Database factory with centralized `ServerConfig`
    ///
    /// # Errors
    ///
    /// Returns an error if database connection or pool configuration fails
    pub async fn new(
        database_url: &str,
        encryption_key: Vec<u8>,
        pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        Self::new_impl(database_url, encryption_key, pool_config).await
    }
}

#[async_trait]
impl DatabaseProvider for PostgresDatabase {
    async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        // Use default pool configuration when called through trait
        // In practice, the Database factory calls the inherent impl's new() directly with config
        let pool_config = PostgresPoolConfig::default();
        Self::new_impl(database_url, encryption_key, &pool_config).await
    }

    async fn migrate(&self) -> AppResult<()> {
        info!("Running PostgreSQL database migrations...");

        pg_migrator()
            .run(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("PostgreSQL migration failed: {e}")))?;

        info!("PostgreSQL database migrations completed successfully");
        Ok(())
    }
}

// System settings operations, from the one shared body
impl_system_settings!(PostgresDatabase);
