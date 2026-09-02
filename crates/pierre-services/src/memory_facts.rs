// ABOUTME: User-facing memory fact service — list and forget what the coach remembers
// ABOUTME: Wraps HarnessMemoryRepository with user-scoped wire shapes for the GDPR Forget UX
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-facing memory facts service.
//!
//! Exposes a tightly-scoped read+delete surface for [`pierre_memory::UserFact`]
//! rows so the user-facing memory panel can show what the coach remembers
//! and let the user GDPR-forget any individual fact. Tenant ownership is
//! enforced by the caller (the route handler resolves the active tenant
//! from the authenticated session before invoking these helpers).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::CoachRepos;
use pierre_memory::FactKind;

/// Default page size when the client omits `limit`. Bounded to 100 by
/// [`MAX_LIST_LIMIT`] so a misconfigured client cannot drag the database.
pub const DEFAULT_LIST_LIMIT: i64 = 50;
/// Maximum number of facts returned in a single response.
pub const MAX_LIST_LIMIT: i64 = 100;

/// Wire shape for a single stored user fact. Mirrors the domain
/// [`pierre_memory::UserFact`] but flattens enums to stable string keys
/// so the `TypeScript` client can render them without an extra mapping.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserFactRow {
    /// Stable identifier — the key the Forget action uses.
    pub id: String,
    /// Coach the fact is scoped to, or `null` for cross-coach facts.
    pub coach_id: Option<String>,
    /// The title of that coach, resolved for the panel so it can name the
    /// coach rather than print its id; `null` when the fact has no coach or
    /// the coach no longer resolves for this user.
    pub coach_title: Option<String>,
    /// The `FactKind` serde name — `preference`, `physiology`, `injury`,
    /// `goal`, `schedule`, `equipment`, `north_star`, `medical` or `other`.
    pub kind: String,
    /// Subject phrase, typically `"you"`.
    pub subject: String,
    /// Verb phrase (`prefers`, `has`, `runs`, etc.).
    pub predicate: String,
    /// Object phrase — the fact value the user can review.
    pub object: String,
    /// Confidence in `[0.0, 1.0]` from the extractor.
    pub confidence: f32,
    /// Source message id for "jump to source" UI affordances.
    pub source_msg_id: Option<String>,
    /// RFC3339 timestamp of the most recent update to this fact.
    pub updated_at: String,
}

/// Response envelope for `GET /api/memory/facts`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserFactListResponse {
    /// Facts ordered most-recently-updated first.
    pub facts: Vec<UserFactRow>,
    /// Total number of facts returned in this response.
    pub total: usize,
}

/// Response envelope for `DELETE /api/memory/facts/:fact_id`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ForgetFactResponse {
    /// `true` when a row was removed, `false` when no matching fact was
    /// found (still a success — the desired post-condition holds).
    pub deleted: bool,
}

/// Parse a `snake_case` fact-kind filter into a [`FactKind`] enum value.
#[must_use]
pub fn fact_kind_from_query(raw: Option<&str>) -> Option<FactKind> {
    raw.map(FactKind::parse_lenient)
}

/// List the authenticated user's stored facts, optionally filtered by
/// coach and/or kind.
///
/// `limit` is clamped to `1..=100`; callers should default to
/// [`DEFAULT_LIST_LIMIT`] when the client omits the parameter.
///
/// # Errors
///
/// Returns repository errors propagated from
/// [`pierre_database::repositories::HarnessMemoryRepository::list_user_facts`].
pub async fn list_user_facts(
    repos: &CoachRepos,
    tenant_id: TenantId,
    user_id: &str,
    coach_id: Option<&str>,
    kind: Option<FactKind>,
    limit: i64,
) -> AppResult<UserFactListResponse> {
    let clamped = limit.clamp(1, MAX_LIST_LIMIT);
    let facts = repos
        .memory
        .list_user_facts(tenant_id, user_id, coach_id, kind, clamped)
        .await?;

    let coach_titles = coach_titles_for(repos, &facts, user_id, tenant_id).await;

    let rows: Vec<UserFactRow> = facts
        .into_iter()
        .map(|f| UserFactRow {
            id: f.id,
            coach_title: f
                .coach_id
                .as_deref()
                .and_then(|id| coach_titles.get(id).cloned()),
            coach_id: f.coach_id,
            kind: f.kind.as_str().to_owned(),
            subject: f.subject,
            predicate: f.predicate,
            object: f.object,
            confidence: f.confidence,
            source_msg_id: f.source_msg_id,
            updated_at: f.updated_at.to_rfc3339(),
        })
        .collect();

    let total = rows.len();
    Ok(UserFactListResponse { facts: rows, total })
}

/// The title of every coach the facts name, one lookup per distinct coach.
///
/// A page of facts usually names one or two coaches many times over, so the
/// lookups are keyed by coach id rather than run per row. A coach that no
/// longer resolves for this user — deleted, or from a tenant the user left —
/// simply has no title, and the row keeps its id.
async fn coach_titles_for(
    repos: &CoachRepos,
    facts: &[pierre_memory::UserFact],
    user_id: &str,
    tenant_id: TenantId,
) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    let Ok(user_uuid) = Uuid::parse_str(user_id) else {
        return titles;
    };
    let mut distinct: Vec<&str> = facts.iter().filter_map(|f| f.coach_id.as_deref()).collect();
    distinct.sort_unstable();
    distinct.dedup();
    for coach_id in distinct {
        if let Ok(Some(coach)) = repos
            .coaches
            .get_by_id(coach_id, user_uuid, tenant_id)
            .await
        {
            titles.insert(coach_id.to_owned(), coach.title);
        }
    }
    titles
}

/// GDPR-grade Forget: remove a single fact when it belongs to the
/// authenticated user. Returns `Ok(false)` when no row matched (idempotent —
/// the post-condition is "this fact is gone").
///
/// # Errors
///
/// Returns repository errors propagated from
/// [`pierre_database::repositories::HarnessMemoryRepository::delete_user_fact`].
pub async fn forget_user_fact(
    repos: &CoachRepos,
    fact_id: &str,
    tenant_id: TenantId,
    user_id: &str,
) -> AppResult<ForgetFactResponse> {
    let deleted = repos
        .memory
        .delete_user_fact(fact_id, tenant_id, user_id)
        .await?;
    Ok(ForgetFactResponse { deleted })
}
