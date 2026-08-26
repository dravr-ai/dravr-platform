// ABOUTME: PostgreSQL writes for the single-column preferences hanging off the users row
// ABOUTME: Analytics consent, locale, coaching persona, roster flag, timezone and theme
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Preference writes for the `PostgreSQL` backend.
//!
//! Each of these sets exactly one column on `users` and reports a missing row
//! as `NotFound` rather than a silent no-op — a preference the client believes
//! it saved and the server quietly dropped is worse than an error. They are
//! grouped here because they are the same statement six times over, and
//! reading one should not mean scrolling through the account lifecycle.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::CoachingPersona;
use sqlx::{Pool, Postgres};
use uuid::Uuid;

/// Turn "no row matched" into the `NotFound` the callers contract on.
fn ensure_updated(rows_affected: u64, user_id: Uuid) -> AppResult<()> {
    if rows_affected == 0 {
        return Err(AppError::not_found(format!("User with ID: {user_id}")));
    }
    Ok(())
}

/// Update the user's analytics-consent preference, stamping the decision time.
///
/// # Errors
///
/// Returns an error if the user is not found or the database update fails.
pub async fn update_analytics_consent(
    pool: &Pool<Postgres>,
    user_id: Uuid,
    enabled: bool,
) -> AppResult<()> {
    let result = sqlx::query(
        r"
        UPDATE users SET
            analytics_consent = $1,
            analytics_consent_at = CURRENT_TIMESTAMP
        WHERE id = $2
        ",
    )
    .bind(enabled)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to update analytics consent: {e}")))?;

    ensure_updated(result.rows_affected(), user_id)
}

/// Update the user's preferred locale.
///
/// # Errors
///
/// Returns an error if the user is not found or the database update fails.
pub async fn update_locale(pool: &Pool<Postgres>, user_id: Uuid, locale: &str) -> AppResult<()> {
    let result = sqlx::query("UPDATE users SET locale = $1 WHERE id = $2")
        .bind(locale)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user locale: {e}")))?;

    ensure_updated(result.rows_affected(), user_id)
}

/// Set the user's coaching persona (output format / cadence preference).
///
/// Persisted as `snake_case` enum text — the column has
/// `NOT NULL DEFAULT 'casual'` and the application-side [`CoachingPersona`]
/// enum is the source of truth for the allowed value set.
///
/// # Errors
///
/// Returns an error if the user is not found or the database update fails.
pub async fn set_coaching_persona(
    pool: &Pool<Postgres>,
    user_id: Uuid,
    persona: CoachingPersona,
) -> AppResult<()> {
    let result = sqlx::query("UPDATE users SET coaching_persona = $1 WHERE id = $2")
        .bind(persona.as_str())
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set coaching persona: {e}")))?;

    ensure_updated(result.rows_affected(), user_id)
}

/// Set whether the user manages a coaching roster.
///
/// # Errors
///
/// Returns an error if the user is not found or the database update fails.
pub async fn set_manages_roster(
    pool: &Pool<Postgres>,
    user_id: Uuid,
    manages_roster: bool,
) -> AppResult<()> {
    let result = sqlx::query("UPDATE users SET manages_roster = $1 WHERE id = $2")
        .bind(manages_roster)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set manages_roster: {e}")))?;

    ensure_updated(result.rows_affected(), user_id)
}

/// Set the user's IANA timezone.
///
/// # Errors
///
/// Returns an error if the user is not found or the database update fails.
pub async fn set_timezone(pool: &Pool<Postgres>, user_id: Uuid, timezone: &str) -> AppResult<()> {
    let result = sqlx::query("UPDATE users SET timezone = $1 WHERE id = $2")
        .bind(timezone)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set timezone: {e}")))?;

    ensure_updated(result.rows_affected(), user_id)
}

/// Pin, or clear, the user's colour scheme.
///
/// `Some("light")` / `Some("dark")` pin the scheme across every device;
/// `None` clears the pin so clients follow the operating system and
/// server-side chart renders fall back to dark.
///
/// # Errors
///
/// Returns an error if the user is not found or the database update fails.
pub async fn set_theme(pool: &Pool<Postgres>, user_id: Uuid, theme: Option<&str>) -> AppResult<()> {
    let result = sqlx::query("UPDATE users SET theme = $1 WHERE id = $2")
        .bind(theme)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set theme: {e}")))?;

    ensure_updated(result.rows_affected(), user_id)
}
