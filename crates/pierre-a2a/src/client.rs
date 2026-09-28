// ABOUTME: A2A client registration, management, and lifecycle operations
// ABOUTME: Manages client credentials, usage statistics, and rate limiting for A2A agents
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A Client Management
//!
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - Arc resource sharing for A2A client management
// - String ownership for client IDs, names, and API keys
//!
//! Handles registration, management, and monitoring of A2A clients
//! that connect to Pierre for agent-to-agent communication.

pub use crate::client_types::{
    A2ARateLimitStatus, ClientCredentials, ClientRegistrationRequest, ClientUsageStats, DailyUsage,
};
use crate::constants::rate_limits::DEFAULT_BURST_LIMIT;
use crate::constants::time::HOUR_SECONDS;
use crate::system_user::A2ASystemUserService;
use crate::{map_db_error, A2AError};
use chrono::{DateTime, Datelike, Days, NaiveTime, Utc};
use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_auth::crypto::A2AKeyManager;
use pierre_auth::rate_limiting::a2a_client_window_start;
pub use pierre_core::models::a2a::{A2AClient, A2ASession, A2AUsage};
// Trait methods are dispatched through repos.a2a / repos.api_keys Arc<dyn Trait>;
use pierre_database::AuthRepos;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use uuid::Uuid;

/// The calendar days, today included, a client's daily usage breakdown
/// covers.
pub const USAGE_HISTORY_DAYS: u32 = 30;

/// A2A Client Manager
pub struct A2AClientManager {
    /// Narrow view over `users`-adjacent repositories — `A2AClientManager`
    /// only ever touches `repos.a2a` (client lifecycle, sessions, usage) and
    /// `repos.api_keys` (key issuance during registration).
    repos: AuthRepos,
    system_user_service: Arc<A2ASystemUserService>,
    active_sessions: Arc<RwLock<HashMap<String, A2ASession>>>,
}

impl A2AClientManager {
    /// Creates a new A2A client manager instance.
    ///
    /// Takes the narrow `AuthRepos` view — the manager only consults
    /// `repos.a2a` (client lifecycle, sessions, usage) and
    /// `repos.api_keys` (key issuance during registration).
    #[must_use]
    pub fn new(repos: &AuthRepos, system_user_service: Arc<A2ASystemUserService>) -> Self {
        Self {
            repos: repos.clone(),
            system_user_service,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new A2A client
    ///
    /// # Arguments
    ///
    /// * `request` - The client registration request details
    /// * `user_id` - The ID of the user registering the client (for ownership tracking)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Registration request validation fails
    /// - Keypair generation fails
    /// - System user creation fails
    /// - Database storage fails
    #[allow(clippy::cast_possible_truncation)] // Safe: HOUR_SECONDS is 3600, well within u32 range
    pub async fn register_client(
        &self,
        request: ClientRegistrationRequest,
        user_id: Uuid,
    ) -> Result<ClientCredentials, A2AError> {
        // Validate registration request
        Self::validate_registration_request(&request)?;

        // Generate client credentials
        let client_id = format!("a2a_client_{}", Uuid::new_v4());
        let client_secret = format!("a2a_secret_{}", Uuid::new_v4());

        // Generate Ed25519 keypair for the client
        let keypair = A2AKeyManager::generate_keypair()
            .map_err(|e| A2AError::InternalError(format!("Failed to generate keypair: {e}")))?;

        // Create proper system user (not dummy user)
        let system_user_id = self
            .system_user_service
            .create_or_get_system_user(&client_id)
            .await
            .map_err(|e| A2AError::InternalError(format!("Failed to create system user: {e}")))?;

        // Create client record with real public key
        let client = A2AClient {
            id: client_id.clone(),
            user_id, // Use the authenticated user's ID for ownership tracking
            name: request.name.clone(),
            description: request.description.clone(), // Safe: Option<String> ownership for client struct
            public_key: keypair.public_key.clone(),   // Safe: String ownership for client struct
            capabilities: request.capabilities.clone(), // Safe: Vec ownership for client struct
            redirect_uris: request.redirect_uris.clone(),
            // The repository stores it normalized; a blank one is no address.
            contact_email: Some(request.contact_email.trim())
                .filter(|email| !email.is_empty())
                .map(ToOwned::to_owned),
            is_active: true,
            created_at: chrono::Utc::now(),
            permissions: vec!["read_activities".into()], // Default permissions
            rate_limit_requests: DEFAULT_BURST_LIMIT * 10,
            rate_limit_window_seconds: HOUR_SECONDS as u32,
            updated_at: chrono::Utc::now(),
        };

        // Store client in database and retrieve the actual generated API key
        let generated_api_key = self
            .store_client_secure(&client, &client_secret, system_user_id)
            .await?;

        info!(
            client_id = %client_id,
            capabilities = ?request.capabilities,
            "A2A client registered successfully"
        );

        Ok(ClientCredentials {
            client_id,
            client_secret,
            api_key: generated_api_key,
            public_key: keypair.public_key,
            private_key: keypair.private_key,
            key_type: "ed25519".into(),
        })
    }

    /// Validate client registration request
    fn validate_registration_request(request: &ClientRegistrationRequest) -> Result<(), A2AError> {
        if request.name.is_empty() {
            return Err(A2AError::InvalidRequest("Client name is required".into()));
        }

        if request.capabilities.is_empty() {
            return Err(A2AError::InvalidRequest(
                "At least one capability is required".into(),
            ));
        }

        // Validate capabilities are known
        let valid_capabilities = [
            "fitness-data-analysis",
            "activity-intelligence",
            "goal-management",
            "performance-prediction",
            "training-analytics",
            "provider-integration",
        ];

        for capability in &request.capabilities {
            if !valid_capabilities.contains(&capability.as_str()) {
                return Err(A2AError::InvalidRequest(format!(
                    "Unknown capability: {capability}"
                )));
            }
        }

        Ok(())
    }

    /// Store client in database with proper security.
    /// Returns the generated API key so callers can pass it to the registrant.
    async fn store_client_secure(
        &self,
        client: &A2AClient,
        client_secret: &str,
        system_user_id: Uuid,
    ) -> Result<String, A2AError> {
        // Create API key using the proper system user
        let api_key_manager = ApiKeyManager::new();

        let request = CreateApiKeyRequest {
            name: format!("A2A Client: {}", client.name),
            description: Some(format!("API key for A2A client: {}", client.description)),
            tier: ApiKeyTier::Professional, // Default tier for A2A clients
            rate_limit_requests: None,      // Use tier default
            expires_in_days: None,          // No expiration
        };

        let (api_key_obj, generated_key) = api_key_manager
            .create_api_key(system_user_id, request)
            .map_err(|e| A2AError::InternalError(format!("Failed to create API key: {e}")))?;

        // Store the API key in database
        self.repos
            .api_keys
            .create(&api_key_obj)
            .await
            .map_err(|e| A2AError::InternalError(format!("Failed to store API key: {e}")))?;

        debug!(
            api_key_id = %api_key_obj.id,
            "Generated API key for A2A client"
        );

        // Create A2A client entry linked to the API key
        self.repos
            .a2a
            .create_client(client, client_secret, &api_key_obj.id)
            .await
            .map_err(|e| A2AError::InternalError(format!("Failed to create A2A client: {e}")))?;

        info!(
            client_id = %client.id,
            client_name = %client.name,
            system_user_id = %system_user_id,
            api_key_id = %api_key_obj.id,
            "A2A client stored securely in database"
        );
        Ok(generated_key)
    }

    /// Get client by ID
    ///
    /// # Errors
    ///
    /// Returns an error if database query fails
    pub async fn get_client(&self, client_id: &str) -> Result<Option<A2AClient>, A2AError> {
        self.repos
            .a2a
            .get_client(client_id)
            .await
            .map_err(map_db_error("Failed to get A2A client"))
    }

    /// List all registered clients for a specific user
    ///
    /// # Errors
    ///
    /// Returns an error if database query fails
    pub async fn list_clients_for_user(
        &self,
        user_id: &uuid::Uuid,
    ) -> Result<Vec<A2AClient>, A2AError> {
        self.repos
            .a2a
            .list_clients(user_id)
            .await
            .map_err(map_db_error("Failed to list A2A clients"))
    }

    /// List all registered clients (system-wide - admin only)
    ///
    /// # Errors
    ///
    /// Returns an error if database query fails
    pub async fn list_all_clients(&self) -> Result<Vec<A2AClient>, A2AError> {
        // For system-wide listing, we use nil UUID to get all clients
        let system_user_id = uuid::Uuid::nil();
        self.repos
            .a2a
            .list_clients(&system_user_id)
            .await
            .map_err(map_db_error("Failed to list all A2A clients"))
    }

    /// Deactivate a client
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Client does not exist
    /// - Database deactivation fails
    pub async fn deactivate_client(&self, client_id: &str) -> Result<(), A2AError> {
        // First verify the client exists
        self.get_client(client_id)
            .await?
            .ok_or_else(|| A2AError::ClientNotRegistered(client_id.to_owned()))?;

        // Deactivate the client in the database
        self.repos
            .a2a
            .deactivate_client(client_id)
            .await
            .map_err(map_db_error("Failed to deactivate A2A client"))?;

        // Invalidate all active sessions for this client
        if let Err(e) = self.repos.a2a.invalidate_client_sessions(client_id).await {
            error!(
                "Failed to invalidate sessions for client {}: {}",
                client_id, e
            );
            // Continue with deactivation even if session invalidation fails
        }

        // Deactivate associated API keys - this is critical for security
        if let Err(e) = self.repos.a2a.deactivate_client_api_keys(client_id).await {
            error!(
                "Failed to deactivate API keys for client {}: {}",
                client_id, e
            );
            // Continue with deactivation even if API key deactivation fails
        }

        Ok(())
    }

    /// A registered client's calls as its owner reads them, counted over its
    /// `a2a_usage` rows: since the start of the UTC day, since the start of
    /// the UTC month, every call it has made, the latest call, and one
    /// success/error row per UTC day that saw a call over the last
    /// [`USAGE_HISTORY_DAYS`] calendar days, today included, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not registered or a database query
    /// fails
    pub async fn get_client_usage(&self, client_id: &str) -> Result<ClientUsageStats, A2AError> {
        let client = self
            .get_client(client_id)
            .await?
            .ok_or_else(|| A2AError::ClientNotRegistered(client_id.to_owned()))?;
        self.client_usage_at(&client.id, Utc::now()).await
    }

    /// `client_id`'s usage with every window cut from `now`, so today always
    /// lies inside this month, even when the month turns mid-read.
    async fn client_usage_at(
        &self,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<ClientUsageStats, A2AError> {
        let today = now.date_naive();
        let start_of_day = today.and_time(NaiveTime::MIN).and_utc();
        let start_of_month = today
            .with_day(1)
            .ok_or_else(|| A2AError::InternalError("Every month has a first day".to_owned()))?
            .and_time(NaiveTime::MIN)
            .and_utc();

        let today_stats = self
            .repos
            .a2a
            .get_usage_stats(client_id, start_of_day, now)
            .await
            .map_err(map_db_error("Failed to get today's usage"))?;
        let month_stats = self
            .repos
            .a2a
            .get_usage_stats(client_id, start_of_month, now)
            .await
            .map_err(map_db_error("Failed to get this month's usage"))?;
        let lifetime_stats = self
            .repos
            .a2a
            .get_usage_stats(client_id, DateTime::UNIX_EPOCH, now)
            .await
            .map_err(map_db_error("Failed to get the client's total usage"))?;

        let first_day = today
            .checked_sub_days(Days::new(u64::from(USAGE_HISTORY_DAYS.saturating_sub(1))))
            .ok_or_else(|| {
                A2AError::InternalError("Daily usage window precedes the calendar".to_owned())
            })?;
        let history = self
            .repos
            .a2a
            .get_client_usage_history(client_id, first_day.and_time(NaiveTime::MIN).and_utc())
            .await
            .map_err(map_db_error("Failed to get the client's daily usage"))?;

        Ok(ClientUsageStats {
            client_id: client_id.to_owned(),
            requests_today: u64::from(today_stats.total_requests),
            requests_this_month: u64::from(month_stats.total_requests),
            total_requests: u64::from(lifetime_stats.total_requests),
            last_request_at: lifetime_stats.last_request_at,
            daily_usage: history
                .into_iter()
                .map(|(day, success_count, error_count)| DailyUsage {
                    date: day.date_naive(),
                    success_count,
                    error_count,
                })
                .collect(),
        })
    }

    /// Update session activity
    ///
    /// # Errors
    ///
    /// Returns an error if database update fails
    pub async fn update_session_activity(&self, session_token: &str) -> Result<(), A2AError> {
        self.repos
            .a2a
            .update_session_activity(session_token)
            .await
            .map_err(|e| A2AError::InternalError(format!("Failed to update session activity: {e}")))
    }

    /// Get active sessions for a client
    pub async fn get_active_sessions(&self, client_id: &str) -> Vec<A2ASession> {
        // Check cache for active sessions
        let cached_sessions = {
            let sessions = self.active_sessions.read().await;
            sessions
                .values()
                .filter(|session| {
                    session.client_id == client_id && session.expires_at > chrono::Utc::now()
                })
                .cloned()
                .collect::<Vec<A2ASession>>()
        };

        if !cached_sessions.is_empty() {
            return cached_sessions;
        }

        // Query database for active sessions if cache is empty
        match self.repos.a2a.get_active_sessions(client_id).await {
            Ok(db_sessions) => {
                // Update cache with sessions from database
                {
                    let mut cache = self.active_sessions.write().await;
                    for session in &db_sessions {
                        cache.insert(session.id.clone(), session.clone()); // Safe: Session ownership for cache HashMap
                    }
                }
                db_sessions
            }
            Err(e) => {
                error!("Failed to query active sessions from database: {e}");
                vec![]
            }
        }
    }

    /// Clean up expired sessions
    pub const fn cleanup_expired_sessions(&self) {
        // With database storage, expired sessions are automatically filtered out
        // This could trigger a cleanup job if needed
    }

    /// The request budget a client-credentials call from `client_id`
    /// spends: its row's `rate_limit_requests` over a sliding
    /// `rate_limit_window_seconds`, counted over its `a2a_usage` rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not registered or a database query
    /// fails
    pub async fn get_client_rate_limit_status(
        &self,
        client_id: &str,
    ) -> Result<A2ARateLimitStatus, A2AError> {
        let client = self
            .get_client(client_id)
            .await?
            .ok_or_else(|| A2AError::ClientNotRegistered(client_id.to_owned()))?;
        let now = Utc::now();
        let usage = self
            .repos
            .a2a
            .get_client_window_usage(&client.id, a2a_client_window_start(&client, now))
            .await
            .map_err(map_db_error(
                "Failed to read the client's rate-limit window",
            ))?;
        Ok(A2ARateLimitStatus::from_window(&client, &usage, now))
    }
}
