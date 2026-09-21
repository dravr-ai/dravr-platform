// ABOUTME: Core database management with migration system for SQLite and PostgreSQL
// ABOUTME: Handles schema setup, user management, API keys, analytics, and A2A authentication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Agent-to-Agent (A2A) authentication and usage tracking
pub mod a2a;
/// The A2A reaper's bulk fail of tasks a dead instance left non-terminal
pub mod a2a_task_reaper;
/// Historical activity backfills still owed, one per (user, provider), leased to one re-runner
pub mod activity_backfill_jobs;
/// Provider-agnostic activity cache (stale-while-revalidate) persistence
pub mod activity_cache_persistence;
/// Admin token management and authorization
pub mod admin;
/// Agent package artefacts — flavour, skeleton, workouts stored per agent (`SQLite`)
pub mod agent_artefacts;
/// Coaches (custom AI personas) storage and management
pub mod agents;
/// Analytics and usage statistics database operations
pub mod analytics;
/// API key management and validation
pub mod api_keys;
/// Chat conversation and message storage
pub mod chat;
/// Claim verdicts from the bullshit detector pipeline
pub mod claim_verdicts;
/// Coaching group storage, membership, and invite management
pub mod coaching_groups;
/// Athlete commitments (`SQLite`) backing `CommitmentRepository`.
pub mod commitments;
/// Email-verification token management — proving a registered address
pub mod email_verification_tokens;
/// Database error types
pub mod errors;
/// Tenant defaults + per-user overrides (`SQLite`) backing `FeatureFlagsRepository`.
pub mod feature_flags;
/// User fitness configuration storage and retrieval
pub mod fitness_configurations;
/// Guardian pending actions (`SQLite`) backing `GuardianPendingActionsRepository`.
pub mod guardian_actions;
/// Health persistence: data sources, sleep, recovery, health snapshots
pub mod health_persistence;
/// Impersonation session management for super admin user impersonation
pub mod impersonation;
/// LLM credential management and admin config overrides
pub mod llm_credentials;
/// LLM usage tracking for cost analysis and quota enforcement
pub mod llm_usage;
/// MCP Tasks extension handle persistence
pub mod mcp_tasks;
/// Coaching harness memory foundations (facts, compaction, notes, followups, sessions)
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
/// `SQLite` `PlaybookRepository` impl — procedural coaching memory.
pub mod playbooks;
/// Pre-approved email allow-list consulted at registration (`SQLite`)
pub mod pre_approved_emails;
/// Endurance `prescribed_workouts` audit-trail repository (`SQLite`)
pub mod prescribed_workouts;
/// Provider connections: unified connection tracking for all provider types
pub mod provider_connections;
/// Recipe storage and management for nutrition planning
pub mod recipes;
/// Repository trait definitions for focused database access
pub mod repositories;
/// Messaging turns the shutdown drain handed off, leased to one re-runner at a time
pub mod resumable_turns;
/// Agent-athlete roster assignments (`SQLite`) backing `RosterRepository`.
pub mod roster;
/// Endurance cached GPX `route_summaries` repository (`SQLite`)
pub mod route_summaries;
/// Security repository: the RSA signing keypair and the system secrets
pub mod security_repository;
/// Seeder repository for seed-only database operations
pub mod seeder;
/// First-party refresh tokens — the credential a device holds between JWTs
pub mod session_refresh_tokens;
/// URL shortener: `code` → `target_url` with an integer-epoch TTL (`SQLite`)
pub mod short_links;
/// Store listings for agent publishing workflow (`SQLite`)
pub mod store_listings;
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
/// Usage counters for rate limiting and quota enforcement
pub mod usage_counters;
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
/// User account management and authentication
pub mod users;
/// dravr-meteo persistent weather cache (geographic + hourly buckets)
pub mod weather_cache;
/// The periodic-worker ledger, claimed in one statement so two instances tick once
pub mod worker_runs;
/// Endurance user-authored `workout_templates` repository (`SQLite`)
pub mod workout_templates;

/// Test utilities for database operations
pub mod test_utils;

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

use crate::backends::{shared, DatabaseProvider};
use crate::database::migrations::SQLITE_MIGRATIONS;
use base64::engine::general_purpose;
use base64::Engine;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::digest::{digest, SHA256};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use sqlx::{Pool, Sqlite, SqlitePool};
use std::collections::HashMap;
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

        let db = Self {
            pool,
            blind_index_key: encryption_key.clone(),
            active_dek_version: shared::encryption::LEGACY_DEK_VERSION,
            prior_dek_versions: HashMap::new(),
            encryption_key,
        };

        // Run migrations
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
            Err(AppError::internal(format!(
                "No DEK available for ciphertext version {version} (active version is {})",
                self.active_dek_version
            )))
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
        Self::encrypt_data_with_aad_impl(self, data, aad_context)
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
        Self::decrypt_data_with_aad_impl(self, encrypted_data, aad_context)
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

    /// Encrypt sensitive data using AES-256-GCM
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails
    pub fn encrypt_data(&self, data: &str) -> AppResult<String> {
        let rng = SystemRandom::new();

        // Generate unique nonce
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|e| AppError::internal(format!("Failed to generate nonce: {e}")))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        // Create encryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.encryption_key)
            .map_err(|e| AppError::internal(format!("Failed to create encryption key: {e}")))?;
        let key = LessSafeKey::new(unbound_key);

        // Encrypt data
        let mut data_bytes = data.as_bytes().to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut data_bytes)
            .map_err(|e| AppError::internal(format!("Failed to encrypt data: {e}")))?;

        // Combine nonce and encrypted data, base64 encode, then tag with the active DEK version
        let mut combined = nonce_bytes.to_vec();
        combined.extend(data_bytes);

        Ok(shared::encryption::tag_dek_version(
            self.active_dek_version,
            &general_purpose::STANDARD.encode(combined),
        ))
    }

    /// Decrypt sensitive data
    ///
    /// # Errors
    ///
    /// Returns an error if decryption fails or data is malformed
    pub fn decrypt_data(&self, encrypted_data: &str) -> AppResult<String> {
        // Split the DEK version tag, then decode the base64 payload
        let (dek_version, payload) = shared::encryption::split_dek_version(encrypted_data);
        let dek = self.dek_key_for_version(dek_version)?;
        let combined = general_purpose::STANDARD
            .decode(payload)
            .map_err(|e| AppError::internal(format!("Failed to decode base64: {e}")))?;

        if combined.len() < 12 {
            return Err(AppError::internal("Invalid encrypted data: too short"));
        }

        // Extract nonce and encrypted data
        let (nonce_bytes, encrypted_bytes) = combined.split_at(12);
        let nonce = Nonce::assume_unique_for_key(
            nonce_bytes
                .try_into()
                .map_err(|e| AppError::internal(format!("Invalid nonce size: {e}")))?,
        );

        // Create decryption key for the resolved DEK version
        let unbound_key = UnboundKey::new(&AES_256_GCM, dek)
            .map_err(|e| AppError::internal(format!("Failed to create decryption key: {e}")))?;
        let key = LessSafeKey::new(unbound_key);

        // Decrypt data
        let mut decrypted_data = encrypted_bytes.to_vec();
        let decrypted = key
            .open_in_place(nonce, Aad::empty(), &mut decrypted_data)
            .map_err(|e| AppError::internal(format!("Failed to decrypt data: {e}")))?;

        String::from_utf8(decrypted.to_vec()).map_err(|e| {
            AppError::internal(format!("Failed to convert decrypted data to string: {e}"))
        })
    }

    /// Encrypt sensitive data using AES-256-GCM with Additional Authenticated Data (AAD)
    ///
    /// AAD binds the encrypted data to a specific context (tenant|user|provider|table)
    /// preventing ciphertext from being moved between contexts or users.
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails
    fn encrypt_data_with_aad_impl(&self, data: &str, aad_context: &str) -> AppResult<String> {
        let rng = SystemRandom::new();

        // Generate unique nonce
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|e| AppError::internal(format!("Failed to generate nonce: {e}")))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        // Create encryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.encryption_key)
            .map_err(|e| AppError::internal(format!("Failed to create encryption key: {e}")))?;
        let key = LessSafeKey::new(unbound_key);

        // Encrypt data with AAD binding
        let mut data_bytes = data.as_bytes().to_vec();
        let aad = Aad::from(aad_context.as_bytes());
        key.seal_in_place_append_tag(nonce, aad, &mut data_bytes)
            .map_err(|e| AppError::internal(format!("Failed to encrypt data: {e}")))?;

        // Combine nonce and encrypted data, base64 encode, then tag with the active DEK version
        let mut combined = nonce_bytes.to_vec();
        combined.extend(data_bytes);

        Ok(shared::encryption::tag_dek_version(
            self.active_dek_version,
            &general_purpose::STANDARD.encode(combined),
        ))
    }

    /// Decrypt sensitive data using AES-256-GCM with Additional Authenticated Data (AAD)
    ///
    /// The same AAD context used for encryption MUST be provided for successful decryption.
    /// This prevents ciphertext from being moved between contexts.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Decryption fails
    /// - Data is malformed
    /// - AAD context does not match (authentication fails)
    fn decrypt_data_with_aad_impl(
        &self,
        encrypted_data: &str,
        aad_context: &str,
    ) -> AppResult<String> {
        // Split the DEK version tag, then decode the base64 payload
        let (dek_version, payload) = shared::encryption::split_dek_version(encrypted_data);
        let dek = self.dek_key_for_version(dek_version)?;
        let combined = general_purpose::STANDARD
            .decode(payload)
            .map_err(|e| AppError::internal(format!("Failed to decode base64: {e}")))?;

        if combined.len() < 12 {
            return Err(AppError::internal("Invalid encrypted data: too short"));
        }

        // Extract nonce and encrypted data
        let (nonce_bytes, encrypted_bytes) = combined.split_at(12);
        let nonce = Nonce::assume_unique_for_key(
            nonce_bytes
                .try_into()
                .map_err(|e| AppError::internal(format!("Invalid nonce size: {e}")))?,
        );

        // Create decryption key for the resolved DEK version
        let unbound_key = UnboundKey::new(&AES_256_GCM, dek)
            .map_err(|e| AppError::internal(format!("Failed to create decryption key: {e}")))?;
        let key = LessSafeKey::new(unbound_key);

        // Decrypt data with AAD verification
        let mut decrypted_data = encrypted_bytes.to_vec();
        let aad = Aad::from(aad_context.as_bytes());
        let decrypted = key
            .open_in_place(nonce, aad, &mut decrypted_data)
            .map_err(|e| {
                AppError::internal(format!(
                    "Decryption failed (possible AAD mismatch or tampered data): {e:?}"
                ))
            })?;

        String::from_utf8(decrypted.to_vec()).map_err(|e| {
            AppError::internal(format!("Failed to convert decrypted data to string: {e}"))
        })
    }

    /// Compute HMAC-SHA256 of a token for secure storage (internal implementation)
    ///
    /// Used for refresh tokens where we need deterministic lookups but don't
    /// need to recover the original value.
    fn hash_token_for_storage_impl(&self, token: &str) -> String {
        // Pinned to the blind-index key (DEK v1) so lookups survive DEK rotation.
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.blind_index_key);
        let tag = hmac::sign(&key, token.as_bytes());
        general_purpose::STANDARD.encode(tag.as_ref())
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

    /// Hash sensitive data using SHA-256
    ///
    /// # Errors
    ///
    /// Returns an error if hashing fails
    pub fn hash_data(&self, data: &str) -> AppResult<String> {
        let hash = digest(&SHA256, data.as_bytes());
        Ok(general_purpose::STANDARD.encode(hash.as_ref()))
    }
}

// Implement HasEncryption trait for SQLite (delegates to inherent impl methods)
impl shared::encryption::HasEncryption for Database {
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        // Call inherent impl directly to avoid infinite recursion
        Self::encrypt_data_with_aad_impl(self, data, aad)
    }

    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        // Call inherent impl directly to avoid infinite recursion
        Self::decrypt_data_with_aad_impl(self, encrypted, aad)
    }

    fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
        Ok(Self::hash_token_for_storage_impl(self, token))
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
