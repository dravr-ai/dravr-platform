// ABOUTME: System-wide settings an operator can change — auto-approval and social insights
// ABOUTME: Split from admin_ops.rs, which is about acting on users rather than configuring the system
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Global switches, not per-user actions.
//!
//! `admin_ops` is about doing something to a person — approve them, suspend
//! them, change their tier. These functions change how the system behaves for
//! everyone, which is a different blast radius and a different audit story.
//!
//! Two of them are environment-shadowed: when `AUTO_APPROVE_USERS` is set in
//! the process environment it wins over the database row, and the returned
//! [`AutoApprovalSettings`] says so via `overridden_by_env` — so a UI can render
//! the value as fixed instead of accepting a write the next read would discard.

use pierre_config::mcp::AppBehaviorConfig;
use pierre_core::config::social::SocialInsightsConfig;
use pierre_core::errors::AppError;
use pierre_runtime_context::DataContext;
use tracing::error;

/// Resolved auto-approval settings combining env var and database state
pub struct AutoApprovalSettings {
    /// True when new user registrations are auto-approved
    pub enabled: bool,
    /// Email domains that bypass approval regardless of the global flag
    pub auto_approve_domains: Vec<String>,
    /// True when the process environment, not the database, decided `enabled`.
    ///
    /// The database row is then inert: writing it changes nothing until
    /// `AUTO_APPROVE_USERS` is unset and the server restarted, so callers
    /// surface the setting as read-only instead of editable.
    pub overridden_by_env: bool,
}
/// Retrieve the effective auto-approval setting.
///
/// Precedence: env var (if set) > database > default. The returned
/// `overridden_by_env` records which side won, so a caller that offers an
/// editing surface can present the value as fixed rather than accepting a
/// write the next read would discard.
///
/// # Errors
///
/// Returns `Internal` if the database read for the auto-approval flag fails.
pub async fn get_auto_approval_settings(
    data: &DataContext,
    app_behavior: &AppBehaviorConfig,
) -> Result<AutoApprovalSettings, AppError> {
    let enabled = if app_behavior.auto_approve_users_from_env {
        app_behavior.auto_approve_users
    } else {
        match data.database().is_auto_approval_enabled().await {
            Ok(Some(db_setting)) => db_setting,
            Ok(None) => app_behavior.auto_approve_users,
            Err(e) => {
                error!(error = %e, "Failed to get auto-approval setting");
                return Err(AppError::internal(format!(
                    "Failed to get auto-approval setting: {e}"
                )));
            }
        }
    };

    Ok(AutoApprovalSettings {
        enabled,
        auto_approve_domains: app_behavior.auto_approve_domains.clone(),
        overridden_by_env: app_behavior.auto_approve_users_from_env,
    })
}

/// Persist a new auto-approval setting to the database.
///
/// The stored row only governs behaviour while `AUTO_APPROVE_USERS` is absent
/// from the environment; [`get_auto_approval_settings`] is the sole authority
/// on the effective value, so callers that report an outcome to a client read
/// it back from there rather than echoing `enabled`.
///
/// # Errors
///
/// Returns `Internal` if the database write fails.
pub async fn set_auto_approval(data: &DataContext, enabled: bool) -> Result<(), AppError> {
    data.database()
        .set_auto_approval_enabled(enabled)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to set auto-approval setting");
            AppError::internal(format!("Failed to set auto-approval setting: {e}"))
        })
}

// =========================================================================
// Social insights settings
// =========================================================================

/// Retrieve the current social insights configuration.
///
/// # Errors
///
/// Returns `Internal` if the configuration read fails.
pub async fn get_social_insights_config(
    data: &DataContext,
) -> Result<SocialInsightsConfig, AppError> {
    data.database()
        .get_social_insights_config()
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get social insights config");
            AppError::internal(format!("Failed to get social insights config: {e}"))
        })
        .map(Option::unwrap_or_default)
}

/// Persist updated social insights configuration.
///
/// # Errors
///
/// Returns `Internal` if the configuration write fails.
pub async fn set_social_insights_config(
    data: &DataContext,
    config: &SocialInsightsConfig,
) -> Result<(), AppError> {
    data.database()
        .set_social_insights_config(config)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to set social insights config");
            AppError::internal(format!("Failed to set social insights config: {e}"))
        })
}

/// Reset social insights configuration to defaults.
///
/// # Errors
///
/// Returns `Internal` if the configuration deletion fails.
pub async fn reset_social_insights_config(
    data: &DataContext,
) -> Result<SocialInsightsConfig, AppError> {
    data.database()
        .delete_social_insights_config()
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to reset social insights config");
            AppError::internal(format!("Failed to reset social insights config: {e}"))
        })?;

    Ok(SocialInsightsConfig::default())
}

// =========================================================================
// Analytics: recent activity dashboard
// =========================================================================
