// ABOUTME: Handlers for the athlete's answers to their agent's proposals and for « Envoyer » under /api/me/plan
// ABOUTME: Accept moves a proposal live, refuse drops it from the overlay; the send review reads the provider, the send runs the push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1841. What the athlete's agent changes unasked waits in their own
//! overlay, « Proposé », until they answer it; and nothing of their plan
//! reaches their calendar until they send it.
//!
//! - `POST /api/me/plan/proposals/dates/{date}/accept` — the proposal on the
//!   date, and on the other date of a session it moves, put on the live plan
//!   through the one live write that rebases the overlay; idempotent;
//! - `POST /api/me/plan/proposals/dates/{date}/refuse` — the live entries put
//!   back on the date (and its pair) of the overlay; the live plan is never
//!   touched;
//! - `POST /api/me/plan/send/review` — what « Envoyer » would send, checked
//!   against the provider: an entry the athlete edited there since Dravr
//!   wrote it is named `edited_on_provider` and left out of the count;
//! - `POST /api/me/plan/send` — the send itself, the push
//!   `push_training_plan` runs, replacing the edited entries only on the
//!   dates `replace_edited` names.
//!
//! Every one of them is refused to an outside client (403
//! `external_transport`) before anything is read. Accept and refuse answer
//! with the plan as `GET /api/me/plan` reads it after them.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::CivilDate;
use pierre_core::errors::{AppError, ErrorCode, ErrorResponse};
use pierre_memory::training_plans::PlanWeek;
use pierre_middleware::AuthenticatedUser;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{coach_plan_view, CoachDraftTarget};
use pierre_services::own_plan::accept_proposal;
use pierre_tool_runtime::implementations::coach_session_editor::refuse_own_date;
use pierre_tool_runtime::implementations::plan_scope::external_own_write_error;
use pierre_tool_runtime::implementations::training_plan_push::{
    own_pending_send, review_own_send, send_own_plan, OwnSendRefusal,
};
use serde_json::json;

use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::tools::runtime_adapter::into_runtime;

use super::common::refusal;
use super::dto::{PendingSendResponse, SendRequest, SendResponse, SendReviewResponse};
use super::own_plan::{own_caller, own_plan_response, OwnCaller};
use super::subject_session::plan_date;

/// The caller, past the gate every write on their own plan runs: an outside
/// client is refused before anything is read.
async fn own_writer(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
) -> Result<OwnCaller, AppError> {
    if auth.acts_for_outside_client() {
        return Err(external_own_write_error());
    }
    own_caller(resources, auth).await
}

/// The athlete's live weeks, as the send is judged on them.
async fn live_weeks(
    resources: &Arc<ServerContext>,
    caller: &OwnCaller,
) -> Result<Vec<PlanWeek>, AppError> {
    let target = caller.own.target();
    Ok(coach_plan_view(&resources.common.repos, &target)
        .await?
        .live
        .map(|season| season.weeks)
        .unwrap_or_default())
}

/// The status a refused send answers with: 409 to choose a calendar, 422
/// with none to send to, 404 with no plan, 403 for every gate.
fn send_refused(refused: &OwnSendRefusal) -> Response {
    let status = match refused {
        OwnSendRefusal::ChooseCalendar(_) => StatusCode::CONFLICT,
        OwnSendRefusal::NoCalendar => StatusCode::UNPROCESSABLE_ENTITY,
        OwnSendRefusal::NoPlan => StatusCode::NOT_FOUND,
        OwnSendRefusal::CoachManaged
        | OwnSendRefusal::TenantDisabled
        | OwnSendRefusal::Refused { .. } => StatusCode::FORBIDDEN,
    };
    let code = match status {
        StatusCode::NOT_FOUND => ErrorCode::ResourceNotFound,
        StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
        _ => ErrorCode::InvalidInput,
    };
    let mut error = refusal(code, refused.reason(), &refused.message());
    if let OwnSendRefusal::ChooseCalendar(calendars) = refused {
        error.details = Some(Box::new(
            json!({ "reason": refused.reason(), "calendars": calendars }),
        ));
    }
    (status, Json(ErrorResponse::from(error))).into_response()
}

/// `POST /api/me/plan/proposals/dates/{date}/accept`: what the athlete's
/// agent proposed on the date — and on the other date of a session it moves —
/// put on their live plan, answered with the plan as `GET /api/me/plan`
/// reads it after. A date that proposes nothing, already accepted or refused,
/// is answered as the plan stands.
///
/// 400 `invalid_date`; 403 `external_transport`.
pub(super) async fn accept_own_proposal(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(raw_date): Path<String>,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let caller = own_writer(&resources, &auth).await?;
    let date = plan_date(&raw_date)?;
    let own = caller.own;
    accept_proposal(&resources.common.repos, own.tenant, own.athlete, date).await?;
    let response = own_plan_response(&resources, &caller, query.locale.as_deref()).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `POST /api/me/plan/proposals/dates/{date}/refuse`: the live plan's
/// entries put back on the date of the athlete's overlay, and on the other
/// date of a session it moves, answered with the plan as `GET /api/me/plan`
/// reads it after. The live plan is never touched; a date that proposes
/// nothing is answered as the plan stands.
///
/// 400 `invalid_date`; 403 `external_transport`.
pub(super) async fn refuse_own_proposal(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(raw_date): Path<String>,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let caller = own_writer(&resources, &auth).await?;
    let date = plan_date(&raw_date)?;
    let own = caller.own;
    let view = coach_plan_view(
        &resources.common.repos,
        &CoachDraftTarget::own(own.tenant, own.athlete),
    )
    .await?;
    let dates: Vec<CivilDate> = view
        .proposal_groups()
        .into_iter()
        .find(|group| group.contains(&date))
        .unwrap_or_else(|| vec![date]);
    let runtime = into_runtime(&resources);
    for day in dates {
        refuse_own_date(&runtime, &own, day).await?;
    }
    let response = own_plan_response(&resources, &caller, query.locale.as_deref()).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `POST /api/me/plan/send/review`: what « Envoyer » would send of the
/// athlete's live plan, checked against their calendar: each entry says
/// whether they edited it there since Dravr wrote it, and the count leaves
/// those out. Nothing is written.
///
/// 409 `choose_calendar` (with `details.calendars`); 422 `no_calendar`; 403
/// `coach_managed` and the provider's gates; 403 `external_transport`.
pub(super) async fn review_own_plan_send(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let caller = own_writer(&resources, &auth).await?;
    let weeks = live_weeks(&resources, &caller).await?;
    let today = athlete_today(&resources.common.repos, auth.user_id).await;
    let review = match review_own_send(&caller.context, &weeks, today).await? {
        Ok(review) => review,
        Err(refused) => return Ok(send_refused(&refused)),
    };
    let edited_keys = review.edited_on_provider;
    let mut edited_dates: Vec<CivilDate> = review
        .pending
        .entries
        .iter()
        .filter(|key| edited_keys.contains(&key.external_id))
        .map(|key| key.date)
        .collect();
    edited_dates.sort_unstable();
    edited_dates.dedup();
    let mut counted: Vec<CivilDate> = review
        .pending
        .entries
        .iter()
        .filter(|key| !edited_keys.contains(&key.external_id))
        .map(|key| key.date)
        .collect();
    counted.sort_unstable();
    counted.dedup();
    let keys: Vec<bool> = review
        .pending
        .entries
        .iter()
        .map(|key| edited_keys.contains(&key.external_id))
        .collect();
    let mut pending_send = PendingSendResponse::of(review.pending);
    for (entry, edited) in pending_send.entries.iter_mut().zip(keys) {
        entry.edited_on_provider = Some(edited);
    }
    let response = SendReviewResponse {
        pending_send,
        edited_on_provider: edited_dates,
        count: counted.len(),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `POST /api/me/plan/send`: the athlete's « Envoyer » — their live plan put
/// on their calendar, the entries they edited there left alone except on the
/// dates `replace_edited` names — answered with what it created, updated,
/// removed, skipped and failed, and what is pending after it.
///
/// 409 `choose_calendar` (with `details.calendars`); 422 `no_calendar`; 403
/// `tenant_disabled`, `coach_managed` and the provider's gates; 404
/// `no_plan`; 400 `invalid_date`; 403 `external_transport`.
pub(super) async fn send_own_plan_route(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(request): Json<SendRequest>,
) -> Result<Response, AppError> {
    let caller = own_writer(&resources, &auth).await?;
    let replace_edited = request
        .replace_edited
        .iter()
        .map(|raw| plan_date(raw))
        .collect::<Result<Vec<CivilDate>, AppError>>()?;
    let weeks = live_weeks(&resources, &caller).await?;
    let today = athlete_today(&resources.common.repos, auth.user_id).await;
    let report = match send_own_plan(&caller.context, &weeks, today, &replace_edited).await? {
        Ok(report) => report,
        Err(refused) => return Ok(send_refused(&refused)),
    };
    let after = own_pending_send(&caller.context, &weeks, today).await?;
    let response = SendResponse::of(report, PendingSendResponse::of(after));
    Ok((StatusCode::OK, Json(response)).into_response())
}
