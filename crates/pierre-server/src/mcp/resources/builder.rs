// ABOUTME: ServerContextBuilder — fluent builder that fills ServerContextOptions and calls ServerContext::new
// ABOUTME: Production binary + test fixtures use this to avoid manual resource assembly anti-patterns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{ServerContext, ServerContextOptions};
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_cache::Cache;
use pierre_core::billing::BillingProvider;
use pierre_database::backends::factory::Database;
use pierre_llm::ChatProvider;
use pierre_llm::LlmProvider;
use std::sync::Arc;

use pierre_config::environment::ServerConfig;

/// Builder pattern for `ServerContext` to avoid manual resource assembly anti-patterns
pub struct ServerContextBuilder {
    database: Option<Database>,
    auth_manager: Option<AuthManager>,
    admin_jwt_secret: Option<String>,
    config: Option<Arc<ServerConfig>>,
    cache: Option<Cache>,
    rsa_key_size_bits: usize,
    jwks_manager: Option<Arc<JwksManager>>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    chat_provider: Option<Arc<ChatProvider>>,
    billing_provider: Option<Arc<dyn BillingProvider>>,
}

impl ServerContextBuilder {
    /// Create a new builder with production defaults (4096-bit RSA keys)
    #[must_use]
    pub const fn new() -> Self {
        Self {
            database: None,
            auth_manager: None,
            admin_jwt_secret: None,
            config: None,
            cache: None,
            rsa_key_size_bits: 4096, // Production default
            jwks_manager: None,
            llm_provider: None,
            chat_provider: None,
            billing_provider: None,
        }
    }

    /// Set the database
    #[must_use]
    pub fn with_database(mut self, database: Database) -> Self {
        self.database = Some(database);
        self
    }

    /// Set the auth manager
    #[must_use]
    pub const fn with_auth_manager(mut self, auth_manager: AuthManager) -> Self {
        self.auth_manager = Some(auth_manager);
        self
    }

    /// Set the admin JWT secret
    #[must_use]
    pub fn with_admin_jwt_secret(mut self, admin_jwt_secret: impl Into<String>) -> Self {
        self.admin_jwt_secret = Some(admin_jwt_secret.into());
        self
    }

    /// Set the server configuration
    #[must_use]
    pub fn with_config(mut self, config: Arc<ServerConfig>) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the cache
    #[must_use]
    pub fn with_cache(mut self, cache: Cache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set the RSA key size for JWT signing (2048 for tests, 4096 for production)
    #[must_use]
    pub const fn with_rsa_key_size_bits(mut self, rsa_key_size_bits: usize) -> Self {
        self.rsa_key_size_bits = rsa_key_size_bits;
        self
    }

    /// Set a pre-existing JWKS manager (for test performance - reuses RSA keys)
    #[must_use]
    pub fn with_jwks_manager(mut self, jwks_manager: Arc<JwksManager>) -> Self {
        self.jwks_manager = Some(jwks_manager);
        self
    }

    /// Set an LLM provider for insight validation (for testing with mock providers)
    #[must_use]
    pub fn with_llm_provider(mut self, llm_provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(llm_provider);
        self
    }

    /// Set the pre-built [`ChatProvider`] singleton (used by the production
    /// binary so chat / coach / social / memory / health-probe consumers
    /// share one warm provider instance instead of rebuilding per call).
    #[must_use]
    pub fn with_chat_provider(mut self, chat_provider: Arc<ChatProvider>) -> Self {
        self.chat_provider = Some(chat_provider);
        self
    }

    /// Set the billing provider backing `/api/billing/*` and
    /// `/webhooks/{provider}`. When unset, resources init falls back to the
    /// in-tree `DummyProvider`.
    #[must_use]
    pub fn with_billing_provider(mut self, billing_provider: Arc<dyn BillingProvider>) -> Self {
        self.billing_provider = Some(billing_provider);
        self
    }

    /// Build the `ServerContext`
    ///
    /// # Errors
    ///
    /// Returns an error if any required fields are missing
    pub async fn build(self) -> Result<ServerContext, &'static str> {
        let database = self.database.ok_or("Database is required")?;
        let auth_manager = self.auth_manager.ok_or("AuthManager is required")?;
        let admin_jwt_secret = self
            .admin_jwt_secret
            .ok_or("Admin JWT secret is required")?;
        let config = self.config.ok_or("Server config is required")?;
        let cache = self.cache.ok_or("Cache is required")?;

        let options = ServerContextOptions {
            rsa_key_size_bits: Some(self.rsa_key_size_bits),
            jwks_manager: self.jwks_manager,
            llm_provider: self.llm_provider,
            chat_provider: self.chat_provider,
            extra_tools: Vec::new(),
            billing_provider: self.billing_provider,
            turn_runner: None,
        };

        let resources = ServerContext::new(
            database,
            auth_manager,
            &admin_jwt_secret,
            config,
            cache,
            options,
        )
        .await;
        Ok(resources)
    }

    /// Build the `ServerContext` wrapped in an `Arc`
    ///
    /// # Errors
    ///
    /// Returns an error if any required fields are missing
    pub async fn build_arc(self) -> Result<Arc<ServerContext>, &'static str> {
        let resources = Arc::new(self.build().await?);
        Ok(resources)
    }
}

impl Default for ServerContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}
