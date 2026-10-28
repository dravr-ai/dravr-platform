// ABOUTME: HTTP boundary for a coach's review of the agent's group replies — the page of replies, a flag, a correction, the weekly count
// ABOUTME: A correction to a reply sent in a bound Telegram/Slack/Discord chat also reaches that chat through the chat poster (carnet#847)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for a coach's after-the-fact review of group replies (carnet#847).
//!
//! - `GET /api/coach/replies` — every reply the agent sent into a group the
//!   caller coaches, newest first, narrowed by `athlete_id` or `group_id` and
//!   paged back by `before`, each with its verdict chips, the figures the
//!   checker could not settle and the caller's own flags and corrections.
//! - `POST /api/coach/replies/{entry_id}/flags` — flag a reply (`wrong`,
//!   `unsafe` or `off_tone`, with a note). The athlete never sees it.
//! - `POST /api/coach/replies/{entry_id}/corrections` — post a correction. It
//!   joins the group thread linked to the reply, and when the agent sent the
//!   reply in the group's bound messaging chat the chat gets it too, where its
//!   athletes read the reply.
//! - `GET /api/coach/replies/weekly` — the caller's flags and corrections per
//!   week.
//!
//! The rules live in [`pierre_services::coach_replies`]; this module is the
//! HTTP boundary plus the chat post, which needs the messaging adapters.
//!
//! No route resolves the caller's tenant: a coach reviews the groups they
//! coach whatever tenant owns them — an athlete's group that invited them
//! lives in the athlete's tenant (carnet#1210) — and every review row is
//! scoped to its group's own tenant by the repository.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use pierre_core::errors::AppError;
use pierre_core::models::{CoachReplyCorrection, CoachReplyFlag, CoachReviewWeek, ReplyFlagReason};
use pierre_core::uuid_utils::parse_uuid;
use pierre_middleware::AuthenticatedUser;
use pierre_services::coach_replies::{self, ReplyPageQuery};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Query of `GET /api/coach/replies`.
#[derive(Debug, Default, Deserialize)]
pub struct RepliesQuery {
    /// Only replies to this athlete
    #[serde(default)]
    pub athlete_id: Option<String>,
    /// Only replies sent into this group
    #[serde(default)]
    pub group_id: Option<String>,
    /// The oldest reply the caller already holds: the page holds the replies
    /// sent before it
    #[serde(default)]
    pub before: Option<String>,
    /// Replies per page, clamped to `1..=50`
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Body of `POST /api/coach/replies/{entry_id}/flags`.
#[derive(Debug, Deserialize)]
pub struct FlagBody {
    /// Why the reply is flagged
    pub reason: ReplyFlagReason,
    /// The coach's note
    #[serde(default)]
    pub note: Option<String>,
}

/// Body of `POST /api/coach/replies/{entry_id}/corrections`.
#[derive(Debug, Deserialize)]
pub struct CorrectionBody {
    /// The correction, as the group thread will show it
    pub content: String,
}

/// Response of the flag route.
#[derive(Debug, Serialize)]
pub struct FlagResponse {
    /// The flag just raised
    pub flag: CoachReplyFlag,
}

/// Response of the correction route.
#[derive(Debug, Serialize)]
pub struct CorrectionResponse {
    /// The correction just posted
    pub correction: CoachReplyCorrection,
    /// Whether the group's bound messaging chat accepted it too; `false` for a
    /// group bound to no chat, and for a reply the agent did not send in it
    pub posted_to_chat: bool,
}

/// Query of `GET /api/coach/replies/weekly`.
#[derive(Debug, Default, Deserialize)]
pub struct WeeklyQuery {
    /// Weeks to cover, the current one included, clamped to `1..=26`
    #[serde(default)]
    pub weeks: Option<u32>,
}

/// Response of the weekly count.
#[derive(Debug, Serialize)]
pub struct WeeklyResponse {
    /// One row per week, oldest first
    pub weeks: Vec<CoachReviewWeek>,
}

/// A query id, parsed; a malformed one is invalid input.
fn optional_id(raw: Option<&str>) -> Result<Option<Uuid>, AppError> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_uuid)
        .transpose()
}

/// `GET /api/coach/replies` — one page of the caller's review.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, an id is malformed, or a read
/// fails.
pub async fn handle_list(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<RepliesQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let page = coach_replies::list_replies(
        &resources.common.repos,
        &resources.mcp.evidence_registry,
        auth.user_id,
        ReplyPageQuery {
            group_id: optional_id(query.group_id.as_deref())?,
            athlete_user_id: optional_id(query.athlete_id.as_deref())?,
            before: optional_id(query.before.as_deref())?,
            limit: query.limit,
        },
    )
    .await?;
    Ok((StatusCode::OK, Json(page)).into_response())
}

/// `POST /api/coach/replies/{entry_id}/flags` — flag one reply.
///
/// # Errors
///
/// Returns not-found for a reply the caller may not review, invalid input for
/// a malformed id or an over-long note, and `AppError` when the write fails.
pub async fn handle_flag(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(entry_id): Path<String>,
    Json(body): Json<FlagBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let outcome = coach_replies::flag_reply(
        &resources.common.repos,
        auth.user_id,
        parse_uuid(&entry_id)?,
        body.reason,
        body.note.as_deref(),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(FlagResponse { flag: outcome.flag }),
    )
        .into_response())
}

/// `POST /api/coach/replies/{entry_id}/corrections` — post a correction into
/// the group thread, and into the group's bound chat when the reply was sent
/// there.
///
/// # Errors
///
/// Returns not-found for a reply the caller may not review, invalid input for
/// a malformed id or an empty or over-long correction, and `AppError` when the
/// write fails. A chat that refuses the post fails nothing: the correction is
/// already in the thread, and `posted_to_chat` says it did not reach the chat.
pub async fn handle_correction(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(entry_id): Path<String>,
    Json(body): Json<CorrectionBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let outcome = coach_replies::correct_reply(
        &resources.common.repos,
        auth.user_id,
        parse_uuid(&entry_id)?,
        &body.content,
    )
    .await?;
    // Without the messaging adapters there is no chat to post into: the
    // correction lives in the group thread only.
    #[cfg(all(feature = "client-messaging", feature = "client-groups"))]
    let posted_to_chat = chat_post::post(&resources, auth.user_id, &outcome).await;
    #[cfg(not(all(feature = "client-messaging", feature = "client-groups")))]
    let posted_to_chat = false;
    Ok((
        StatusCode::CREATED,
        Json(CorrectionResponse {
            correction: outcome.correction,
            posted_to_chat,
        }),
    )
        .into_response())
}

/// `GET /api/coach/replies/weekly` — the caller's flags and corrections per week.
///
/// # Errors
///
/// Returns `AppError` when authentication fails or the read fails.
pub async fn handle_weekly(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<WeeklyQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let weeks = coach_replies::weekly_counts(
        &resources.common.repos,
        auth.user_id,
        query.weeks,
        Utc::now(),
    )
    .await?;
    Ok((StatusCode::OK, Json(WeeklyResponse { weeks })).into_response())
}

/// A correction posted into a group's bound messaging chat.
#[cfg(all(feature = "client-messaging", feature = "client-groups"))]
mod chat_post;

/// Mount-helper for the coach review routes.
#[derive(Debug)]
pub struct CoachReplyRoutes;

impl CoachReplyRoutes {
    /// Coach-facing routes. Require a valid session; who may review which
    /// reply is decided per reply by [`pierre_services::coach_replies`].
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/coach/replies", get(handle_list))
            .route("/api/coach/replies/weekly", get(handle_weekly))
            .route("/api/coach/replies/{entry_id}/flags", post(handle_flag))
            .route(
                "/api/coach/replies/{entry_id}/corrections",
                post(handle_correction),
            )
            .with_state(resources)
    }
}
