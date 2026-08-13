// ABOUTME: PostgreSQL link-state lifecycle — the one-time codes binding a channel identity to a user
// ABOUTME: Free functions over the pool; the MessagingRepository impl in messaging.rs delegates here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A link state is the one-time code that turns "someone messaged us from this
//! Telegram/Slack id" into "this id belongs to that Dravr account".
//!
//! Mirrors `database/messaging_link_states.rs` on the `SQLite` side, and exists
//! for the same reason: the lifecycle is a concern of its own, unrelated to the
//! channel configs, sessions, messages and outbound queue that share the table
//! prefix in `messaging.rs`.
//!
//! These are free functions over a `&PgPool` rather than a second `impl` block,
//! because `MessagingRepository` is a trait impl and Rust does not allow one to
//! be split across modules. The trait methods in `messaging.rs` are one-line
//! delegations, which keeps the dispatch surface intact and the SQL out of an
//! already-oversized file.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::repositories::CreateLinkStateParams;

pub(crate) async fn create_link_state(
    pool: &PgPool,
    params: &CreateLinkStateParams<'_>,
) -> AppResult<()> {
    let now = Utc::now();

    // Parse the expires_at string into a DateTime for PostgreSQL TIMESTAMPTZ
    let expires_at: DateTime<Utc> = chrono::DateTime::parse_from_rfc3339(params.expires_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::invalid_input(format!("Invalid expires_at timestamp: {e}")))?;

    // Casts: migration 20260417000001 converted tenant_id and user_id to
    // UUID. SQL casts ($2::uuid, $3::uuid) bridge the wire types so the
    // call sites can keep binding via TenantId/String.
    sqlx::query(
        r"
        INSERT INTO messaging_link_states
            (id, tenant_id, user_id, channel_type, code, method, used,
             channel_user_id, sender_name, expires_at, created_at)
        VALUES ($1, $2::uuid, $3::uuid, $4, $5, $6, FALSE, $7, $8, $9, $10)
        ",
    )
    .bind(params.id)
    .bind(params.tenant_id.to_string())
    .bind(params.user_id)
    .bind(params.channel_type)
    .bind(params.code)
    .bind(params.method)
    .bind(params.channel_user_id)
    .bind(params.sender_name)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to create link state: {e}")))?;

    Ok(())
}

/// Atomically consume a link state by verification code.
///
/// Uses SELECT then atomic `UPDATE` with `used = FALSE` guard and expiry check,
/// verifying `rows_affected` to ensure one-time use.
pub(crate) async fn consume_link_state(
    pool: &PgPool,
    code: &str,
    tenant_id: TenantId,
) -> AppResult<Value> {
    use pierre_core::errors::messaging::MessagingError;

    let now = Utc::now();
    let tenant_id_str = tenant_id.to_string();

    // Check if the code exists for this tenant (before attempting consumption).
    // Casts: migration 20260417000001 stores tenant_id/user_id as UUID.
    let existing = sqlx::query(
        r"
        SELECT id,
               tenant_id::text AS tenant_id,
               user_id::text   AS user_id,
               channel_type, code, method, used,
               channel_user_id, sender_name, expires_at, created_at
        FROM messaging_link_states
        WHERE code = $1 AND tenant_id = $2::uuid
        ",
    )
    .bind(code)
    .bind(&tenant_id_str)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

    let Some(row) = existing else {
        return Err(MessagingError::LinkCodeExpired.into());
    };

    let used: bool = row.get("used");
    if used {
        return Err(MessagingError::LinkCodeAlreadyUsed.into());
    }

    let expires_at: DateTime<Utc> = row.get("expires_at");
    if expires_at < now {
        return Err(MessagingError::LinkCodeExpired.into());
    }

    // Atomic consumption: UPDATE with guards (including tenant_id)
    let result = sqlx::query(
        r"
        UPDATE messaging_link_states
        SET used = TRUE
        WHERE code = $1 AND tenant_id = $2::uuid AND used = FALSE AND expires_at > $3
        ",
    )
    .bind(code)
    .bind(&tenant_id_str)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to consume link state: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(MessagingError::LinkCodeAlreadyUsed.into());
    }

    let created_at: DateTime<Utc> = row.get("created_at");

    Ok(serde_json::json!({
        "id": row.get::<String, _>("id"),
        "tenant_id": row.get::<String, _>("tenant_id"),
        "user_id": row.try_get::<Option<String>, _>("user_id").ok().flatten(),
        "channel_type": row.get::<String, _>("channel_type"),
        "code": row.get::<String, _>("code"),
        "method": row.get::<String, _>("method"),
        "channel_user_id": row.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
        "sender_name": row.try_get::<Option<String>, _>("sender_name").ok().flatten(),
        "expires_at": expires_at.to_rfc3339(),
        "created_at": created_at.to_rfc3339(),
    }))
}

pub(crate) async fn get_link_state(pool: &PgPool, code: &str) -> AppResult<Option<Value>> {
    let now = Utc::now();

    // Cast tenant_id/user_id UUID columns back to text for json serialization.
    let row = sqlx::query(
        r"
        SELECT id,
               tenant_id::text AS tenant_id,
               user_id::text   AS user_id,
               channel_type, code, method, used,
               channel_user_id, sender_name, expires_at, created_at
        FROM messaging_link_states
        WHERE code = $1 AND used = FALSE AND expires_at > $2
        ",
    )
    .bind(code)
    .bind(now)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

    Ok(row.map(|r| {
        let expires_at: DateTime<Utc> = r.get("expires_at");
        let created_at: DateTime<Utc> = r.get("created_at");
        serde_json::json!({
            "id": r.get::<String, _>("id"),
            "tenant_id": r.get::<String, _>("tenant_id"),
            "user_id": r.try_get::<Option<String>, _>("user_id").ok().flatten(),
            "channel_type": r.get::<String, _>("channel_type"),
            "code": r.get::<String, _>("code"),
            "method": r.get::<String, _>("method"),
            "channel_user_id": r.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
            "sender_name": r.try_get::<Option<String>, _>("sender_name").ok().flatten(),
            "expires_at": expires_at.to_rfc3339(),
            "created_at": created_at.to_rfc3339(),
        })
    }))
}

pub(crate) async fn complete_link_state(
    pool: &PgPool,
    code: &str,
    user_id: &str,
) -> AppResult<Value> {
    use pierre_core::errors::messaging::MessagingError;

    let now = Utc::now();

    let existing = sqlx::query(
        r"
        SELECT id,
               tenant_id::text AS tenant_id,
               user_id::text   AS user_id,
               channel_type, code, method, used,
               channel_user_id, sender_name, expires_at, created_at
        FROM messaging_link_states
        WHERE code = $1
        ",
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

    let Some(row) = existing else {
        return Err(MessagingError::LinkCodeExpired.into());
    };

    let used: bool = row.get("used");
    if used {
        return Err(MessagingError::LinkCodeAlreadyUsed.into());
    }

    let expires_at: DateTime<Utc> = row.get("expires_at");
    if expires_at < now {
        return Err(MessagingError::LinkCodeExpired.into());
    }

    let existing_user_id: Option<String> =
        row.try_get::<Option<String>, _>("user_id").ok().flatten();
    if existing_user_id.is_some() {
        return Err(MessagingError::LinkCodeNotCompletable {
            code: code.to_owned(),
            reason: "Link code already has a user_id set".to_owned(),
        }
        .into());
    }

    let result = sqlx::query(
        r"
        UPDATE messaging_link_states
        SET user_id = $1::uuid, used = TRUE
        WHERE code = $2 AND used = FALSE AND user_id IS NULL AND expires_at > $3
        ",
    )
    .bind(user_id)
    .bind(code)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to complete link state: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(MessagingError::LinkCodeAlreadyUsed.into());
    }

    let created_at: DateTime<Utc> = row.get("created_at");

    Ok(serde_json::json!({
        "id": row.get::<String, _>("id"),
        "tenant_id": row.get::<String, _>("tenant_id"),
        "user_id": user_id,
        "channel_type": row.get::<String, _>("channel_type"),
        "code": row.get::<String, _>("code"),
        "method": row.get::<String, _>("method"),
        "channel_user_id": row.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
        "sender_name": row.try_get::<Option<String>, _>("sender_name").ok().flatten(),
        "expires_at": expires_at.to_rfc3339(),
        "created_at": created_at.to_rfc3339(),
    }))
}
