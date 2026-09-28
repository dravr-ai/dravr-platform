// ABOUTME: System settings database operations for admin-configurable options
// ABOUTME: The setting type and keys; the SQL is written once in repositories/system_settings.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::system_settings::{
    impl_system_settings, system_setting_from_row, GET_SYSTEM_SETTING_SQL, SET_SYSTEM_SETTING_SQL,
};
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};

/// System setting key constants
pub const SETTING_AUTO_APPROVAL_ENABLED: &str = "auto_approval_enabled";
/// Lifetime, in minutes, of an email-verification link. Operator-scoped:
/// clamped on read to the bounds in `pierre_config::constants::email_verification`.
pub const SETTING_EMAIL_VERIFICATION_TTL_MINUTES: &str = "email_verification_ttl_minutes";
/// Maximum verification emails a single user can trigger per hour. Operator-scoped
/// and clamped on read, so a malformed row cannot disable the throttle.
pub const SETTING_EMAIL_VERIFICATION_MAX_PER_HOUR: &str = "email_verification_max_per_hour";

/// A system setting entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSetting {
    /// Unique key identifier for the setting
    pub key: String,
    /// The current value of the setting
    pub value: String,
    /// Human-readable description of what this setting controls
    pub description: Option<String>,
    /// When the setting was last modified
    pub updated_at: chrono::DateTime<Utc>,
}

impl_system_settings!(Database);
