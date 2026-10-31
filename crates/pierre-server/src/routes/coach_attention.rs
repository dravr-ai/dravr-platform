// ABOUTME: Coach Home's attention reasons on the roster rows, and the coach's "Traité" on one of them (dismiss and restore)
// ABOUTME: The reasons are pierre_services::coach_attention's; this module reads their signals and the stored dismissals (carnet#1252)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Attention reasons on `GET /api/coach/roster`, and their dismissal.
//!
//! Every roster row carries `attention`: the reasons the coach should look at
//! the athlete today or this week, ranked and evidenced by
//! [`pierre_services::coach_attention`]. This module turns a row's snapshot,
//! reading, plan and replies into that module's signals, and holds the two
//! writes a coach makes on a reason:
//!
//! - `POST /api/coach/attention/dismiss` — "Traité": the reason is shown as
//!   handled while its signal's fingerprint is the one dismissed. A changed
//!   signal brings it back.
//! - `POST /api/coach/attention/restore` — undo the dismissal.
//!
//! A dismissal is stored under the athlete's roster tenant and keyed by the
//! coach, the athlete and the reason id; only an athlete on the caller's
//! roster can be named.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::{DateTime, SubsecRound, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::MemberFitnessSnapshot;
use pierre_core::models::TenantId;
use pierre_database::repositories::AttentionDismissalKey;
use pierre_database::RepositoryRegistry;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::backend_resolver;
use pierre_services::coach_attention::TrainingSignals;
use pierre_services::coach_roster::{gather_members, roster_access, OrderedMember, RosterAccess};
use pierre_tool_runtime::coach_roster::RosterReading;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::coach_roster::not_a_coach;

/// Longest reason id or fingerprint a dismissal accepts. The server's own
/// are a kind and an id or an instant; anything longer is not one of them.
const MAX_KEY_LEN: usize = 200;

/// The reasons the coach dismissed, per athlete: reason id to the
/// fingerprint and instant of the dismissal.
pub(crate) type PageDismissals = HashMap<Uuid, BTreeMap<String, (String, DateTime<Utc>)>>;

/// The instant each source last delivered a session, in name order.
#[must_use]
pub(crate) fn sources_of(snapshot: &MemberFitnessSnapshot) -> BTreeMap<String, DateTime<Utc>> {
    snapshot
        .last_activity_per_provider
        .iter()
        .map(|(source, at)| (source.clone(), *at))
        .collect()
}

/// When each source the athlete must reconnect started refusing access, by
/// the name the reading gives it, where the connection recorded the instant.
/// Read only for an athlete with a source to reconnect.
///
/// # Errors
///
/// Returns the repository error from the connection read.
pub(crate) async fn refused_since(
    repos: &RepositoryRegistry,
    athlete: Uuid,
    reading: Option<&RosterReading>,
) -> AppResult<BTreeMap<String, DateTime<Utc>>> {
    let mut since = BTreeMap::new();
    if reading.is_none_or(|r| r.needs_reconnect.is_empty()) {
        return Ok(since);
    }
    for connection in repos
        .provider_connections
        .get_for_user(athlete, None)
        .await?
    {
        let Some(at) = connection
            .status_changed_at
            .filter(|_| connection.status.requires_reauth())
        else {
            continue;
        };
        // A mirror backend (`sciotte_garmin`) is the athlete's Garmin, as the
        // reading names it; the latest flag of the pair wins.
        let name = backend_resolver::user_facing_name(&connection.provider).to_owned();
        let entry = since.entry(name).or_insert(at);
        *entry = (*entry).max(at);
    }
    Ok(since)
}

/// A sharer's training, as the reasons read it.
#[must_use]
pub(crate) fn training_signals<'a>(
    snapshot: &'a MemberFitnessSnapshot,
    reading: &'a RosterReading,
    sources: &'a BTreeMap<String, DateTime<Utc>>,
    refused_since: &'a BTreeMap<String, DateTime<Utc>>,
) -> TrainingSignals<'a> {
    TrainingSignals {
        computed_at: snapshot.computed_at,
        form_as_of: reading
            .form
            .as_ref()
            .map_or_else(|| snapshot.computed_at.date_naive(), |form| form.as_of),
        form_band: reading.form.as_ref().map(|form| form.band),
        days_since_last_activity: reading.days_since_last_activity,
        last_activity_per_source: sources,
        needs_reconnect: &reading.needs_reconnect,
        refused_since,
        stale: reading.stale,
        deep_fatigue_mornings: &snapshot.deep_fatigue_mornings,
        last_session: reading
            .last_session
            .as_ref()
            .map(|s| (s.sport.as_str(), s.duration_seconds)),
    }
}

/// The caller's dismissals for the athletes of `page`, each read under the
/// athlete's own roster tenant.
///
/// # Errors
///
/// Returns the repository error from a read.
pub(crate) async fn page_dismissals(
    repos: &RepositoryRegistry,
    coach: Uuid,
    page: &[OrderedMember],
) -> AppResult<PageDismissals> {
    let tenant_of: HashMap<Uuid, TenantId> = page
        .iter()
        .map(|m| (m.member.user_id, m.member.tenant))
        .collect();
    let tenants: BTreeSet<Uuid> = tenant_of.values().map(TenantId::as_uuid).collect();
    let mut dismissals = PageDismissals::new();
    for tenant in tenants {
        let tenant = TenantId::from_uuid(tenant);
        for row in repos
            .coach_attention_dismissals
            .list_attention_dismissals(tenant, coach)
            .await?
        {
            // Only the athlete's own roster tenant speaks for them.
            if tenant_of.get(&row.athlete_user_id) == Some(&tenant) {
                dismissals
                    .entry(row.athlete_user_id)
                    .or_default()
                    .insert(row.reason_id, (row.fingerprint, row.dismissed_at));
            }
        }
    }
    Ok(dismissals)
}

/// Body of `POST /api/coach/attention/dismiss`.
#[derive(Debug, Deserialize)]
pub struct DismissRequest {
    /// The athlete the reason is about.
    pub athlete_user_id: Uuid,
    /// The reason's `id` on the roster row.
    pub reason_id: String,
    /// The reason's `fingerprint` on the roster row: the signal handled.
    pub fingerprint: String,
}

/// Body of `POST /api/coach/attention/restore`.
#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
    /// The athlete the reason is about.
    pub athlete_user_id: Uuid,
    /// The reason's `id` on the roster row.
    pub reason_id: String,
}

/// Answer to a dismissal.
#[derive(Debug, Serialize)]
pub struct DismissResponse {
    /// The reason dismissed.
    pub reason_id: String,
    /// The signal it was dismissed on.
    pub fingerprint: String,
    /// When.
    pub dismissed_at: DateTime<Utc>,
}

/// Answer to a restore.
#[derive(Debug, Serialize)]
pub struct RestoreResponse {
    /// The reason restored.
    pub reason_id: String,
    /// Whether it had been dismissed.
    pub restored: bool,
}

/// Mount-helper for the attention routes.
#[derive(Debug)]
pub struct CoachAttentionRoutes;

impl CoachAttentionRoutes {
    /// `POST /api/coach/attention/dismiss` and `/restore`. Requires a valid
    /// session; only a coach who may read a roster may write to it.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/coach/attention/dismiss", post(handle_dismiss))
            .route("/api/coach/attention/restore", post(handle_restore))
            .with_state(resources)
    }
}

/// A reason id or fingerprint as a dismissal accepts it.
fn bounded<'a>(field: &str, value: &'a str) -> AppResult<&'a str> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_KEY_LEN {
        return Err(AppError::invalid_input(format!(
            "{field} must be 1 to {MAX_KEY_LEN} characters"
        )));
    }
    Ok(value)
}

/// The roster tenant of `athlete` on the caller's roster.
async fn athlete_tenant(
    repos: &RepositoryRegistry,
    coach: Uuid,
    athlete: Uuid,
) -> AppResult<TenantId> {
    let RosterAccess::Granted(groups) = roster_access(repos, coach).await? else {
        return Err(not_a_coach());
    };
    gather_members(repos, coach, &groups)
        .await?
        .members
        .iter()
        .find(|m| m.user_id == athlete)
        .map(|m| m.tenant)
        .ok_or_else(|| AppError::not_found("Athlete on your roster"))
}

/// `POST /api/coach/attention/dismiss` — mark one reason handled.
///
/// # Errors
///
/// 403 when the caller may not read a roster, 404 for an athlete not on it,
/// invalid input for an empty or over-long id or fingerprint; the repository
/// error otherwise.
pub async fn handle_dismiss(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(body): Json<DismissRequest>,
) -> Result<Response, AppError> {
    let coach = auth.into_inner().user_id;
    let reason_id = bounded("reason_id", &body.reason_id)?;
    let fingerprint = bounded("fingerprint", &body.fingerprint)?;
    let repos = &resources.common.repos;
    let tenant = athlete_tenant(repos, coach, body.athlete_user_id).await?;
    // PostgreSQL keeps six fractional digits and SQLite the text whole, so the
    // instant is truncated to what both store: the answer then matches the row
    // every later roster read returns (as `llm_usage::recorded_at` does).
    let dismissed_at = Utc::now().trunc_subsecs(6);
    repos
        .coach_attention_dismissals
        .dismiss_attention(
            AttentionDismissalKey {
                tenant,
                coach_user_id: coach,
                athlete_user_id: body.athlete_user_id,
                reason_id,
            },
            fingerprint,
            dismissed_at,
        )
        .await?;
    info!(coach = %coach, athlete = %body.athlete_user_id, reason = reason_id, "coach attention dismissed");
    Ok((
        StatusCode::OK,
        Json(DismissResponse {
            reason_id: reason_id.to_owned(),
            fingerprint: fingerprint.to_owned(),
            dismissed_at,
        }),
    )
        .into_response())
}

/// `POST /api/coach/attention/restore` — undo a dismissal.
///
/// # Errors
///
/// 403 when the caller may not read a roster, 404 for an athlete not on it,
/// invalid input for an empty or over-long id; the repository error
/// otherwise.
pub async fn handle_restore(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(body): Json<RestoreRequest>,
) -> Result<Response, AppError> {
    let coach = auth.into_inner().user_id;
    let reason_id = bounded("reason_id", &body.reason_id)?;
    let repos = &resources.common.repos;
    let tenant = athlete_tenant(repos, coach, body.athlete_user_id).await?;
    let restored = repos
        .coach_attention_dismissals
        .restore_attention(AttentionDismissalKey {
            tenant,
            coach_user_id: coach,
            athlete_user_id: body.athlete_user_id,
            reason_id,
        })
        .await?;
    info!(coach = %coach, athlete = %body.athlete_user_id, reason = reason_id, restored, "coach attention restored");
    Ok((
        StatusCode::OK,
        Json(RestoreResponse {
            reason_id: reason_id.to_owned(),
            restored,
        }),
    )
        .into_response())
}
