// ABOUTME: The operator's bounds on self-serve sign-up: kill switch, 24-hour ceiling, activation on a proven address
// ABOUTME: Read from system_settings per sign-up, so tripping the switch needs no deploy (carnet#1063)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What lets a stranger create an account, and what bounds it.
//!
//! Three operator knobs in `system_settings`:
//!
//! - **open** — the kill switch. Closed, every self-serve door refuses: the
//!   public register form, the messaging link page, in-chat sign-up and a
//!   first Google sign-in. An operator's own doors (the admin register route,
//!   an invitation) stay open, since an operator vouches for those accounts.
//! - **daily limit** — the most accounts self-serve sign-up admits in any 24
//!   hours. Counted from the `users` table, whatever created the rows, because
//!   that is the one count every replica and every deploy agrees on.
//! - **activate on a verified email** — when on, proving the address
//!   activates an account no auto-approval rule admitted. This is how sign-up
//!   opens without `AUTO_APPROVE_USERS`, which admits an address nobody proved.
//!
//! None has an environment tier: the row is the only authority, so an
//! operator trips the switch from the console or `pierre-cli settings signup`
//! and every replica reads it on the next sign-up.

use chrono::{Duration, Utc};
use pierre_config::constants::signup as defaults;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::backends::factory::Database;
use pierre_database::database::system_settings::{
    SETTING_SIGNUP_ACTIVATE_ON_VERIFIED_EMAIL, SETTING_SIGNUP_DAILY_LIMIT, SETTING_SIGNUP_OPEN,
};
use pierre_database::repositories::UserRepository;
use tracing::{error, warn};

use crate::email_verification::resolve_clamped;

/// What a self-serve sign-up refused by the kill switch or the ceiling reads.
/// It names neither: whoever is refused only needs to know to come back.
pub const SIGNUP_PAUSED: &str = "Sign-up is paused right now. Please try again later.";

/// The refusal of a sign-up the operator's bounds do not admit.
fn paused() -> AppError {
    AppError::new(ErrorCode::QuotaExceeded, SIGNUP_PAUSED)
}

/// The sign-up bounds in force right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignupPolicy {
    /// Whether self-serve sign-up admits new accounts
    pub open: bool,
    /// The most accounts admitted in any 24 hours; zero sets no ceiling
    pub daily_limit: i64,
    /// Whether proving the address activates an account no rule admitted
    pub activate_on_verified_email: bool,
}

impl Default for SignupPolicy {
    fn default() -> Self {
        Self {
            open: true,
            daily_limit: defaults::DEFAULT_DAILY_LIMIT,
            activate_on_verified_email: false,
        }
    }
}

/// The fields an operator changes; `None` leaves a field as stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SignupPolicyUpdate {
    /// The kill switch
    pub open: Option<bool>,
    /// The 24-hour ceiling, zero for none
    pub daily_limit: Option<i64>,
    /// Activation on a proven address
    pub activate_on_verified_email: Option<bool>,
}

/// Read one boolean setting, `default` when the row is absent, unreadable or
/// neither `true` nor `false`.
async fn resolve_flag(db: &Database, key: &str, default: bool) -> bool {
    match db.get_system_setting(key).await {
        Ok(Some(setting)) => match setting.value.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => {
                warn!(
                    setting = key,
                    "system setting is not a boolean; using the compiled default"
                );
                default
            }
        },
        Ok(None) => default,
        Err(e) => {
            warn!(setting = key, error = %e, "failed to read system setting; using the compiled default");
            default
        }
    }
}

/// Resolve the sign-up bounds in force right now.
///
/// Never fails. A setting that cannot be read takes its default: the switch
/// stays open, because a database hiccup must not close sign-up by itself, and
/// activation stays off, so the same hiccup never opens it either.
pub async fn resolve_signup_policy(db: &Database) -> SignupPolicy {
    SignupPolicy {
        open: resolve_flag(db, SETTING_SIGNUP_OPEN, true).await,
        daily_limit: resolve_clamped(
            db,
            SETTING_SIGNUP_DAILY_LIMIT,
            defaults::DEFAULT_DAILY_LIMIT,
            defaults::MIN_DAILY_LIMIT,
            defaults::MAX_DAILY_LIMIT,
        )
        .await,
        activate_on_verified_email: resolve_flag(
            db,
            SETTING_SIGNUP_ACTIVATE_ON_VERIFIED_EMAIL,
            false,
        )
        .await,
    }
}

/// Store the fields `update` names.
///
/// # Errors
///
/// Returns an invalid-input error for a ceiling outside
/// `[MIN_DAILY_LIMIT, MAX_DAILY_LIMIT]`, before anything is written, and an
/// internal error when a write fails.
pub async fn set_signup_policy(db: &Database, update: SignupPolicyUpdate) -> AppResult<()> {
    if let Some(limit) = update.daily_limit {
        if !(defaults::MIN_DAILY_LIMIT..=defaults::MAX_DAILY_LIMIT).contains(&limit) {
            return Err(AppError::invalid_input(format!(
                "daily_limit must be between {} and {} (0 sets no ceiling)",
                defaults::MIN_DAILY_LIMIT,
                defaults::MAX_DAILY_LIMIT
            )));
        }
    }
    let writes = [
        (SETTING_SIGNUP_OPEN, update.open.map(|v| v.to_string())),
        (
            SETTING_SIGNUP_DAILY_LIMIT,
            update.daily_limit.map(|v| v.to_string()),
        ),
        (
            SETTING_SIGNUP_ACTIVATE_ON_VERIFIED_EMAIL,
            update.activate_on_verified_email.map(|v| v.to_string()),
        ),
    ];
    for (key, value) in writes {
        let Some(value) = value else { continue };
        db.set_system_setting(key, &value).await.map_err(|e| {
            error!(setting = key, error = %e, "Failed to store sign-up setting");
            AppError::internal(format!("Failed to store sign-up setting {key}: {e}"))
        })?;
    }
    Ok(())
}

/// How many accounts were created in the last 24 hours.
///
/// # Errors
///
/// Returns the repository's error when the count fails.
pub async fn created_last_24h(users: &dyn UserRepository) -> AppResult<i64> {
    users
        .count_created_since(Utc::now() - Duration::hours(24))
        .await
}

/// Whether `policy` admits one more self-serve sign-up, with `recent` accounts
/// created in the last 24 hours: refused when the switch is closed or the
/// ceiling is reached.
#[must_use]
pub const fn admits(policy: &SignupPolicy, recent: i64) -> bool {
    policy.open && (policy.daily_limit == 0 || recent < policy.daily_limit)
}

/// Admit one self-serve sign-up, or refuse it.
///
/// A count that fails admits the sign-up: the account write that follows
/// fails on the same database anyway, and a refusal would misreport an outage
/// as an operator's pause.
///
/// # Errors
///
/// Returns a quota-exceeded error (429) reading [`SIGNUP_PAUSED`] when the
/// switch is closed or the ceiling is reached: a refusal the client is told
/// about in words, and no server fault.
pub async fn admit_self_signup(db: &Database, users: &dyn UserRepository) -> AppResult<()> {
    let policy = resolve_signup_policy(db).await;
    if !policy.open {
        warn!("Self-serve sign-up refused: sign-up is closed by an operator");
        return Err(paused());
    }
    if policy.daily_limit == 0 {
        return Ok(());
    }
    let recent = match created_last_24h(users).await {
        Ok(recent) => recent,
        Err(e) => {
            error!(error = %e, "Could not count recent accounts; admitting the sign-up");
            return Ok(());
        }
    };
    if admits(&policy, recent) {
        return Ok(());
    }
    warn!(
        recent,
        daily_limit = policy.daily_limit,
        "Self-serve sign-up refused: the 24-hour ceiling is reached"
    );
    Err(paused())
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn policy(open: bool, daily_limit: i64) -> SignupPolicy {
        SignupPolicy {
            open,
            daily_limit,
            activate_on_verified_email: false,
        }
    }

    #[test]
    fn a_closed_switch_refuses_whatever_the_count() {
        assert!(!admits(&policy(false, 0), 0));
        assert!(!admits(&policy(false, 100), 0));
    }

    #[test]
    fn the_ceiling_admits_up_to_its_limit_and_no_further() {
        assert!(admits(&policy(true, 3), 2));
        assert!(!admits(&policy(true, 3), 3));
        assert!(!admits(&policy(true, 3), 4));
    }

    #[test]
    fn a_zero_ceiling_sets_none() {
        assert!(admits(&policy(true, 0), i64::MAX));
    }

    #[test]
    fn the_default_is_open_bounded_and_not_self_activating() {
        let default = SignupPolicy::default();
        assert!(default.open);
        assert_eq!(default.daily_limit, defaults::DEFAULT_DAILY_LIMIT);
        assert!(!default.activate_on_verified_email);
    }
}
