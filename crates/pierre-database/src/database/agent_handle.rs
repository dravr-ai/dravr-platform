// ABOUTME: Catalogue handle assignment on SQLite — gives an agent its @handle on Store approval or creation
// ABOUTME: Shared by the StoreListingsManager and the direct StoreListingsRepository impl
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AgentHandle, TenantId};
use sqlx::{Row, SqliteConnection};

/// Upper bound on numbered candidates tried before giving up on a title.
const MAX_HANDLE_ATTEMPTS: u32 = 100;

/// Give an agent its catalogue handle, if it does not own one yet — the agent
/// being approved into the Store, or the one `/agent create` just created.
///
/// An origin agent that already carries a handle (a seeded agent) keeps it.
/// Any other agent — a custom agent with no handle, or a copy that inherited
/// its origin's handle as a reference — is assigned the first free candidate
/// derived from its title (`title`, then `title-2`, `title-3`, …). "Free"
/// is judged at catalogue scope: no origin agent and no published agent may
/// already answer to it. Origin rows are additionally guarded by the
/// `idx_agents_handle` unique index, so a concurrent approval of the same
/// candidate fails loudly on the second `UPDATE` instead of producing twins.
///
/// Runs inside the approval transaction so an agent never ends up published
/// without an addressable name, and on a plain connection for a created
/// agent.
///
/// # Errors
///
/// Returns an error when the agent does not exist in the tenant, when every
/// candidate is taken, or when the database fails.
pub async fn ensure_catalogue_handle(
    conn: &mut SqliteConnection,
    agent_id: &str,
    tenant_id: TenantId,
) -> AppResult<String> {
    let row =
        sqlx::query("SELECT title, slug, forked_from FROM agents WHERE id = $1 AND tenant_id = $2")
            .bind(agent_id)
            .bind(tenant_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| AppError::database(format!("Failed to read coach for handle: {e}")))?
            .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;
    let title: String = row.get("title");
    let owned: Option<String> = row.try_get("slug").ok().flatten();
    let forked_from: Option<String> = row.try_get("forked_from").ok().flatten();
    if let (Some(handle), None) = (owned, forked_from) {
        return Ok(handle);
    }

    let base = AgentHandle::derive(&title);
    for attempt in 0..MAX_HANDLE_ATTEMPTS {
        let candidate = base.candidate(attempt);
        let taken = sqlx::query(
            "SELECT 1 FROM agents WHERE slug = $1 AND id <> $2 AND (forked_from IS NULL \
             OR id IN (SELECT agent_id FROM store_listings WHERE publish_status = 'published')) \
             LIMIT 1",
        )
        .bind(candidate.as_str())
        .bind(agent_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| AppError::database(format!("Failed to probe coach handle: {e}")))?;
        if taken.is_some() {
            continue;
        }
        sqlx::query("UPDATE agents SET slug = $1 WHERE id = $2")
            .bind(candidate.as_str())
            .bind(agent_id)
            .execute(&mut *conn)
            .await
            .map_err(|e| AppError::database(format!("Failed to assign coach handle: {e}")))?;
        return Ok(candidate.as_str().to_owned());
    }
    Err(AppError::invalid_input(format!(
        "No free catalogue handle derived from '{title}' after {MAX_HANDLE_ATTEMPTS} candidates"
    )))
}
