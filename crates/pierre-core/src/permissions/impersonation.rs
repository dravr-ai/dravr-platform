// ABOUTME: Impersonation system for admins to act as other users
// ABOUTME: Provides audit logging and session management for impersonation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{AppError, AppResult, ErrorCode};
use crate::models::User;

/// Impersonation session record for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpersonationSession {
    /// Unique session identifier
    pub id: String,
    /// Admin performing the impersonation
    pub impersonator_id: Uuid,
    /// User being impersonated
    pub target_user_id: Uuid,
    /// Reason for impersonation (for audit)
    pub reason: Option<String>,
    /// When impersonation started
    pub started_at: DateTime<Utc>,
    /// When impersonation ended (None if still active)
    pub ended_at: Option<DateTime<Utc>>,
    /// Whether session is currently active
    pub is_active: bool,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// The assistant connection the session was started through over MCP or
    /// A2A, and the only one it changes: an OAuth application's client id,
    /// or [`API_KEY_BINDING`] for the admin's API keys. `None` for a session
    /// the admin console started on the web (carnet#1511).
    #[serde(default)]
    pub bound_client: Option<String>,
}

/// [`ImpersonationSession::bound_client`] of a session an admin started
/// through one of their API keys.
pub const API_KEY_BINDING: &str = "api_key";

/// Whether `impersonator` may act as `target`, on every door a session
/// starts from (the admin console and an admin's assistant).
///
/// # Errors
///
/// Returns an invalid-input error when `target` is the impersonator, and a
/// permission-denied error when `target` is an admin: acting as one would
/// lend its every power to a session the audit trail attributes to someone
/// else.
pub fn check_target(impersonator: Uuid, target: &User) -> AppResult<()> {
    if target.id == impersonator {
        return Err(AppError::invalid_input("Cannot impersonate yourself"));
    }
    if target.role.is_admin() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Cannot impersonate another admin",
        ));
    }
    Ok(())
}

impl ImpersonationSession {
    /// Create a new impersonation session
    #[must_use]
    pub fn new(impersonator_id: Uuid, target_user_id: Uuid, reason: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            impersonator_id,
            target_user_id,
            reason,
            started_at: now,
            ended_at: None,
            is_active: true,
            created_at: now,
            bound_client: None,
        }
    }

    /// A session an admin's assistant starts through the connection
    /// `bound_client` names ([`Self::bound_client`]).
    #[must_use]
    pub fn bound_to(
        impersonator_id: Uuid,
        target_user_id: Uuid,
        reason: Option<String>,
        bound_client: String,
    ) -> Self {
        Self {
            bound_client: Some(bound_client),
            ..Self::new(impersonator_id, target_user_id, reason)
        }
    }

    /// End the impersonation session
    pub fn end(&mut self) {
        self.ended_at = Some(Utc::now());
        self.is_active = false;
    }

    /// Get duration of impersonation in seconds
    #[must_use]
    pub fn duration_seconds(&self) -> i64 {
        let end = self.ended_at.unwrap_or_else(Utc::now);
        (end - self.started_at).num_seconds()
    }
}
