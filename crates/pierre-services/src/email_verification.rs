// ABOUTME: Operator-tunable settings for the email-verification flow, read from system_settings
// ABOUTME: Clamps every stored value to a documented range so a bad row degrades instead of disabling the gate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Runtime settings for email-address verification.
//!
//! The link lifetime and the per-hour send cap are operator knobs, resolved per
//! use from `system_settings` with the compiled defaults as the fallback — the
//! same store `AUTO_APPROVE_USERS` reads, and deliberately **not** the runtime
//! configuration catalog: that surface is per-user and per-tenant overridable,
//! and a user who can lengthen their own verification window or lift their own
//! send throttle has escalated a privilege, not expressed a preference.
//!
//! ## Why every read clamps
//!
//! `system_settings.value` is TEXT, so a row can hold `"abc"`, `"0"`, or
//! `"99999999"`. An unclamped read turns any of those into a disabled gate: a
//! zero TTL kills every link on arrival, a zero cap locks users out of their own
//! accounts permanently, and a decade-long TTL retires the expiry. Parsing
//! failures fall back to the default; out-of-range values are pulled to the
//! nearest bound rather than rejected, so a fat-fingered admin value degrades to
//! something workable instead of breaking signup.
//!
//! ## No environment tier (yet)
//!
//! `AUTO_APPROVE_USERS` resolves env → stored row → default because a deployment
//! must be able to force it regardless of database state. These two knobs carry
//! no such requirement today, so they resolve from the stored row alone. That is
//! a subset of the same precedence, not a competing mechanism: the stored row
//! stays canonical either way.
//!
//! Adding the tier later is cheap and local — put the fields on
//! `AppBehaviorConfig` (it derives `Default`, so the ~47 exhaustive struct
//! literals among its 68 construction sites are the only ones needing a touch)
//! and check them ahead of the stored row in `resolve_settings` below.

use pierre_config::constants::email_verification as defaults;
use pierre_database::backends::factory::Database;
use pierre_database::database::system_settings::{
    SETTING_EMAIL_VERIFICATION_MAX_PER_HOUR, SETTING_EMAIL_VERIFICATION_TTL_MINUTES,
};

/// Resolved, range-checked settings for one verification operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationSettings {
    /// How long an issued link stays usable, in minutes.
    pub ttl_minutes: i64,
    /// How many verification emails one user may trigger per hour.
    pub max_sends_per_hour: i64,
}

impl Default for VerificationSettings {
    fn default() -> Self {
        Self {
            ttl_minutes: defaults::DEFAULT_LINK_TTL_MINUTES,
            max_sends_per_hour: defaults::DEFAULT_MAX_SENDS_PER_HOUR,
        }
    }
}

/// Read one numeric setting, falling back to `default` when the row is absent,
/// unreadable, or not an integer, then pull it into `[min, max]`.
async fn resolve_clamped(db: &Database, key: &str, default: i64, min: i64, max: i64) -> i64 {
    let stored = match db.get_system_setting(key).await {
        Ok(Some(setting)) => setting.value.trim().parse::<i64>().unwrap_or_else(|_| {
            tracing::warn!(
                setting = key,
                "system setting is not an integer; using the compiled default"
            );
            default
        }),
        Ok(None) => default,
        Err(e) => {
            tracing::warn!(
                setting = key,
                error = %e,
                "failed to read system setting; using the compiled default"
            );
            default
        }
    };

    let clamped = stored.clamp(min, max);
    if clamped != stored {
        tracing::warn!(
            setting = key,
            configured = stored,
            applied = clamped,
            "system setting outside its allowed range; clamped"
        );
    }
    clamped
}

/// Resolve the verification settings in force right now.
///
/// Never fails: every failure mode degrades to the compiled default, because a
/// database hiccup must not be able to stop people confirming their address.
pub async fn resolve_settings(db: &Database) -> VerificationSettings {
    VerificationSettings {
        ttl_minutes: resolve_clamped(
            db,
            SETTING_EMAIL_VERIFICATION_TTL_MINUTES,
            defaults::DEFAULT_LINK_TTL_MINUTES,
            defaults::MIN_LINK_TTL_MINUTES,
            defaults::MAX_LINK_TTL_MINUTES,
        )
        .await,
        max_sends_per_hour: resolve_clamped(
            db,
            SETTING_EMAIL_VERIFICATION_MAX_PER_HOUR,
            defaults::DEFAULT_MAX_SENDS_PER_HOUR,
            defaults::MIN_MAX_SENDS_PER_HOUR,
            defaults::MAX_MAX_SENDS_PER_HOUR,
        )
        .await,
    }
}
