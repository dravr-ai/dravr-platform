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
use crate::{map_db_error, A2AError};
use chrono::{DateTime, Datelike, Days, NaiveTime, Utc};
use pierre_auth::oauth2_server::{ClientRegistrationManager, OAuth2Error};
use pierre_auth::rate_limiting::a2a_client_window_start;
pub use pierre_core::models::a2a::{A2AClient, A2ASession, A2AUsage};
use secrecy::SecretString;
// Trait methods are dispatched through repos.a2a / repos.api_keys Arc<dyn Trait>;
use pierre_database::AuthRepos;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use uuid::Uuid;

/// The calendar days, today included, a client's daily usage breakdown
/// covers.
pub const USAGE_HISTORY_DAYS: u32 = 30;

/// How many capability labels a client may declare.
const MAX_CAPABILITY_LABELS: usize = 16;

/// How long one capability label may be.
const MAX_CAPABILITY_LABEL_CHARS: usize = 64;

/// A2A Client Manager
pub struct A2AClientManager {
    /// Narrow view over `users`-adjacent repositories — `A2AClientManager`
    /// only ever touches `repos.a2a` (client lifecycle, sessions, usage) and
    /// the `client_credentials` registration in `repos.oauth2_server`.
    repos: AuthRepos,
    active_sessions: Arc<RwLock<HashMap<String, A2ASession>>>,
}

impl fmt::Debug for A2AClientManager {
    /// Omits `active_sessions`: its keys and each session's `id` are the
    /// bearer session tokens.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("A2AClientManager")
            .field("repos", &self.repos)
            .finish_non_exhaustive()
    }
}

impl A2AClientManager {
    /// Creates a new A2A client manager instance.
    ///
    /// Takes the narrow `AuthRepos` view — the manager only consults
    /// `repos.a2a` (client lifecycle, sessions, usage) and the
    /// `client_credentials` registration in `repos.oauth2_server`.
    #[must_use]
    pub fn new(repos: &AuthRepos) -> Self {
        Self {
            repos: repos.clone(),
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
    /// - Database storage fails
    #[allow(clippy::cast_possible_truncation)] // Safe: HOUR_SECONDS is 3600, well within u32 range
    pub async fn register_client(
        &self,
        request: ClientRegistrationRequest,
        user_id: Uuid,
    ) -> Result<ClientCredentials, A2AError> {
        // Validate registration request
        Self::validate_registration_request(&request)?;

        // The client's id; its secret is minted by its OAuth2 registration
        // when the client is stored.
        let client_id = format!("a2a_client_{}", Uuid::new_v4());

        let client = A2AClient {
            id: client_id.clone(),
            user_id, // Use the authenticated user's ID for ownership tracking
            name: request.name.clone(),
            description: request.description.clone(), // Safe: Option<String> ownership for client struct
            capabilities: request.capabilities.clone(), // Safe: Vec ownership for client struct
            redirect_uris: request.redirect_uris.clone(),
            // The repository stores it normalized; a blank one is no address.
            contact_email: Some(request.contact_email.trim())
                .filter(|email| !email.is_empty())
                .map(ToOwned::to_owned),
            is_active: true,
            created_at: Utc::now(),
            rate_limit_requests: DEFAULT_BURST_LIMIT * 10,
            rate_limit_window_seconds: HOUR_SECONDS as u32,
            updated_at: Utc::now(),
        };

        // Store the client with its OAuth2 registration
        let client_secret = self.store_client_secure(&client).await?;

        info!(
            client_id = %client_id,
            capabilities = ?request.capabilities,
            "A2A client registered successfully"
        );

        Ok(ClientCredentials {
            client_id,
            client_secret: SecretString::from(client_secret),
        })
    }

    /// Validate client registration request
    fn validate_registration_request(request: &ClientRegistrationRequest) -> Result<(), A2AError> {
        if request.name.is_empty() {
            return Err(A2AError::InvalidRequest("Client name is required".into()));
        }

        // Descriptive labels nothing authorizes on, so any label is acceptable;
        // the bound is here because every call copies them onto its usage row.
        if request.capabilities.len() > MAX_CAPABILITY_LABELS {
            return Err(A2AError::InvalidRequest(format!(
                "At most {MAX_CAPABILITY_LABELS} capability labels are accepted"
            )));
        }
        if request
            .capabilities
            .iter()
            .any(|label| label.chars().count() > MAX_CAPABILITY_LABEL_CHARS)
        {
            return Err(A2AError::InvalidRequest(format!(
                "A capability label is at most {MAX_CAPABILITY_LABEL_CHARS} characters"
            )));
        }

        Ok(())
    }

    /// The `OAuth2` registration manager over the one `oauth2_clients` store,
    /// where every A2A client's secret lives and `/oauth2/token` verifies it.
    fn oauth2_clients(&self) -> ClientRegistrationManager {
        ClientRegistrationManager::new(self.repos.oauth2_server.clone())
    }

    /// Store client in database with proper security.
    ///
    /// The client's `client_credentials` registration is stored first, under
    /// the client's own id, so the secret it returns is the one the card's
    /// `tokenUrl` accepts. When anything after it fails the registration is
    /// deleted again, so no credential outlives a client that was never
    /// stored.
    ///
    /// Returns the client secret so callers can pass it to the registrant.
    async fn store_client_secure(&self, client: &A2AClient) -> Result<String, A2AError> {
        let oauth2_clients = self.oauth2_clients();
        let client_secret = oauth2_clients
            .register_client_credentials_client(&client.id, &client.name)
            .await
            .map_err(|e| {
                A2AError::InternalError(format!(
                    "Failed to register client credentials: {}",
                    oauth2_refusal(&e)
                ))
            })?;

        if let Err(e) = self.repos.a2a.create_client(client).await {
            if let Err(cleanup) = oauth2_clients.delete_client(&client.id).await {
                error!(
                    client_id = %client.id,
                    error = %cleanup,
                    "Failed to delete the client_credentials registration of an A2A client that was not stored"
                );
            }
            return Err(A2AError::InternalError(format!(
                "Failed to create A2A client: {e}"
            )));
        }

        info!(
            client_id = %client.id,
            client_name = %client.name,
            "A2A client stored securely in database"
        );
        Ok(client_secret)
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

    /// List all registered clients (system-wide - admin only)
    ///
    /// # Errors
    ///
    /// Returns an error if database query fails
    pub async fn list_all_clients(&self) -> Result<Vec<A2AClient>, A2AError> {
        // For system-wide listing, we use nil UUID to get all clients
        let system_user_id = Uuid::nil();
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

        // Delete its client_credentials registration, so /oauth2/token refuses
        // it as an unknown client from now on. A client whose registration
        // cannot be deleted is still issued tokens, so this failure is the
        // caller's to hear about; the A2A principal refuses those tokens
        // meanwhile, because the client is inactive.
        self.oauth2_clients()
            .delete_client(client_id)
            .await
            .map_err(map_db_error(
                "Failed to delete the client_credentials registration of an A2A client",
            ))?;

        // Invalidate all active sessions for this client
        if let Err(e) = self.repos.a2a.invalidate_client_sessions(client_id).await {
            error!(
                "Failed to invalidate sessions for client {}: {}",
                client_id, e
            );
            // Continue with deactivation even if session invalidation fails
        }

        // Deactivate any API key linked to the client: registration issues
        // none, and a linked key authenticates until it is retired
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
                .filter(|session| session.client_id == client_id && session.expires_at > Utc::now())
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

/// An `OAuth2` refusal as one line: its error code and description.
fn oauth2_refusal(error: &OAuth2Error) -> String {
    error.error_description.as_ref().map_or_else(
        || error.error.clone(),
        |description| format!("{}: {description}", error.error),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(capabilities: Vec<String>) -> ClientRegistrationRequest {
        ClientRegistrationRequest {
            name: "Agent".to_owned(),
            description: "An agent".to_owned(),
            capabilities,
            redirect_uris: Vec::new(),
            contact_email: "ops@example.com".to_owned(),
        }
    }

    /// Nothing authorizes on a capability, so no list of them may stand in
    /// for an authorization: a client declares what it likes, or nothing.
    #[test]
    fn any_label_or_none_registers() {
        for capabilities in [Vec::new(), vec!["anything-at-all".to_owned()]] {
            assert!(
                A2AClientManager::validate_registration_request(&request(capabilities)).is_ok(),
                "bug: a capability label authorizes nothing, so any label or none registers"
            );
        }
    }

    #[test]
    fn too_many_labels_are_refused() {
        let at_cap = vec!["label".to_owned(); MAX_CAPABILITY_LABELS];
        assert!(
            A2AClientManager::validate_registration_request(&request(at_cap)).is_ok(),
            "bug: a client declaring exactly the cap of labels registers"
        );

        let over_cap = vec!["label".to_owned(); MAX_CAPABILITY_LABELS + 1];
        assert!(
            matches!(
                A2AClientManager::validate_registration_request(&request(over_cap)),
                Err(A2AError::InvalidRequest(_))
            ),
            "bug: one label past the cap is the caller's invalid request"
        );
    }

    #[test]
    fn an_oversized_label_is_refused() {
        let at_cap = vec!["x".repeat(MAX_CAPABILITY_LABEL_CHARS)];
        assert!(
            A2AClientManager::validate_registration_request(&request(at_cap)).is_ok(),
            "bug: a label of exactly the cap in characters registers"
        );

        let over_cap = vec!["x".repeat(MAX_CAPABILITY_LABEL_CHARS + 1)];
        assert!(
            matches!(
                A2AClientManager::validate_registration_request(&request(over_cap)),
                Err(A2AError::InvalidRequest(_))
            ),
            "bug: a label one character past the cap is the caller's invalid request"
        );
    }
}
