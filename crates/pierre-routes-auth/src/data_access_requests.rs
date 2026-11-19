// ABOUTME: The athlete's statutory data access requests: write one, follow its legal deadline, read its letter
// ABOUTME: Own first-party session only, scoped to the athlete in their session's tenant; Dravr never sends the letter

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/user/data-access-requests` (carnet#1646, Owl row 60).
//!
//! A fallback beside `/import`: for a company Dravr cannot import from, or
//! data its self-serve export leaves out, the athlete asks it for their
//! personal information under Quebec's P-39.1 or the GDPR
//! (`pierre_services::data_access_requests`).
//!
//! - `GET` lists the athlete's requests, newest first; `POST` writes a draft
//!   (201) from `controller`, `controller_contact`, `athlete_identifier`,
//!   `law` (`quebec_p39_1` / `gdpr`), `scope` (`access` /
//!   `access_portability`) and `letter_locale` (the athlete's language when
//!   omitted);
//! - `GET /{id}` reads one; `PATCH /{id}` edits a draft;
//! - `GET /{id}/letter?locale=` answers the letter: `subject`, `body`, the
//!   `.txt` `text` and its `filename`, and a `mailto` link when the address
//!   the athlete entered is an email address;
//! - `POST /{id}/sent` (`sent_on`), `/{id}/extension` (`extended_until`, GDPR
//!   only), `/{id}/answered` (`answered_on`), `/{id}/close`.
//!
//! Every view carries the stored `status` and the `derived_state` read from
//! the due date today: `due_soon`, `overdue`, or `null`. A request that is not
//! the athlete's, or not in their session's tenant, is not found. Only the
//! athlete's own first-party session is let in, as for the export.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{DataAccessLaw, DataAccessRequest, DataAccessScope};
use pierre_services::data_access_requests::{
    derived_state, render_letter, DataAccessRequests, DerivedState, DraftEdit, RequestDetails,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::field::Empty;
use uuid::Uuid;

use crate::data_export::{own_session, OwnSession};
use crate::AuthRoutesContext;

/// A request as the athlete reads it.
#[derive(Debug, Serialize)]
pub(crate) struct RequestView {
    id: Uuid,
    reference: String,
    controller: String,
    controller_contact: String,
    athlete_identifier: String,
    law: &'static str,
    scope: &'static str,
    letter_locale: String,
    status: &'static str,
    /// `due_soon`, `overdue`, or `null`: read from the due date today.
    derived_state: Option<&'static str>,
    sent_on: Option<NaiveDate>,
    due_on: Option<NaiveDate>,
    extended_until: Option<NaiveDate>,
    answered_on: Option<NaiveDate>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl RequestView {
    fn of(request: DataAccessRequest, today: NaiveDate) -> Self {
        Self {
            derived_state: derived_state(&request, today).map(DerivedState::as_str),
            id: request.id,
            reference: request.reference,
            controller: request.controller,
            controller_contact: request.controller_contact,
            athlete_identifier: request.athlete_identifier,
            law: request.law.as_str(),
            scope: request.scope.as_str(),
            letter_locale: request.letter_locale,
            status: request.status.as_str(),
            sent_on: request.sent_on,
            due_on: request.due_on,
            extended_until: request.extended_until,
            answered_on: request.answered_on,
            created_at: request.created_at,
            updated_at: request.updated_at,
        }
    }
}

/// What `POST /api/user/data-access-requests` takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBody {
    controller: String,
    controller_contact: String,
    athlete_identifier: String,
    law: String,
    scope: String,
    #[serde(default)]
    letter_locale: Option<String>,
}

/// What `PATCH /api/user/data-access-requests/{id}` takes; every field left
/// out keeps its value.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditBody {
    #[serde(default)]
    controller: Option<String>,
    #[serde(default)]
    controller_contact: Option<String>,
    #[serde(default)]
    athlete_identifier: Option<String>,
    #[serde(default)]
    law: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    letter_locale: Option<String>,
}

/// What `POST …/sent` takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SentBody {
    sent_on: NaiveDate,
}

/// What `POST …/extension` takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionBody {
    extended_until: NaiveDate,
}

/// What `POST …/answered` takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnsweredBody {
    answered_on: NaiveDate,
}

/// What `GET …/letter` takes.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct LetterQuery {
    #[serde(default)]
    locale: Option<String>,
}

/// The law a body names.
fn law(text: &str) -> AppResult<DataAccessLaw> {
    DataAccessLaw::from_stored(text)
        .ok_or_else(|| AppError::invalid_input(format!("Unknown law: {text}")))
}

/// The scope a body names.
fn scope(text: &str) -> AppResult<DataAccessScope> {
    DataAccessScope::from_stored(text)
        .ok_or_else(|| AppError::invalid_input(format!("Unknown scope: {text}")))
}

/// The athlete and their session's tenant; requests are kept under a tenant,
/// which an account in none does not have.
async fn session(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
) -> AppResult<(Uuid, TenantId)> {
    let OwnSession { user_id, tenant_id } = own_session(resources, headers).await?;
    let tenant_id =
        tenant_id.ok_or_else(|| AppError::invalid_input("No tenant for this session"))?;
    Ok((user_id, tenant_id))
}

/// One request's view, as a response with `status`.
fn view(status: StatusCode, request: DataAccessRequest, now: DateTime<Utc>) -> Response {
    (status, Json(RequestView::of(request, now.date_naive()))).into_response()
}

/// `GET /api/user/data-access-requests` — the athlete's requests.
///
/// # Errors
/// An authentication or permission error for anything but the athlete's own
/// session; a database error.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "data_access_requests_list", user_id = Empty)
)]
pub(crate) async fn handle_list(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let today = Utc::now().date_naive();
    let requests: Vec<RequestView> = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .list()
        .await?
        .into_iter()
        .map(|request| RequestView::of(request, today))
        .collect();
    Ok(Json(json!({ "requests": requests })).into_response())
}

/// `POST /api/user/data-access-requests` — write a draft.
///
/// # Errors
/// As [`handle_list`]; invalid input for a missing or malformed field, an
/// unknown law or scope, or a language the platform does not write.
#[tracing::instrument(
    skip(resources, headers, body),
    fields(route = "data_access_requests_create", user_id = Empty)
)]
pub(crate) async fn handle_create(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let letter_locale = match body.letter_locale {
        Some(locale) => locale,
        None => {
            resources
                .repos
                .users
                .get_global(user_id)
                .await?
                .ok_or_else(|| AppError::not_found("User"))?
                .locale
        }
    };
    let details = RequestDetails {
        controller: body.controller,
        controller_contact: body.controller_contact,
        athlete_identifier: body.athlete_identifier,
        law: law(&body.law)?,
        scope: scope(&body.scope)?,
        letter_locale,
    };
    let now = Utc::now();
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .create(&details, now)
        .await?;
    Ok(view(StatusCode::CREATED, request, now))
}

/// `GET /api/user/data-access-requests/{id}` — one of the athlete's requests.
///
/// # Errors
/// As [`handle_list`]; not found for a request that is not theirs.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "data_access_requests_get", user_id = Empty)
)]
pub(crate) async fn handle_get(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .get(id)
        .await?;
    Ok(view(StatusCode::OK, request, Utc::now()))
}

/// `PATCH /api/user/data-access-requests/{id}` — edit a draft.
///
/// # Errors
/// As [`handle_get`]; a conflict once it is no longer a draft; invalid input
/// as [`handle_create`].
#[tracing::instrument(
    skip(resources, headers, body),
    fields(route = "data_access_requests_edit", user_id = Empty)
)]
pub(crate) async fn handle_edit(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<EditBody>,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let edit = DraftEdit {
        controller: body.controller,
        controller_contact: body.controller_contact,
        athlete_identifier: body.athlete_identifier,
        law: body.law.as_deref().map(law).transpose()?,
        scope: body.scope.as_deref().map(scope).transpose()?,
        letter_locale: body.letter_locale,
    };
    let now = Utc::now();
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .edit_draft(id, &edit, now)
        .await?;
    Ok(view(StatusCode::OK, request, now))
}

/// `POST /api/user/data-access-requests/{id}/sent` — the athlete sent it.
///
/// # Errors
/// As [`handle_get`]; a conflict for an answered or closed request; invalid
/// input for a date in the future or more than a year back.
#[tracing::instrument(
    skip(resources, headers, body),
    fields(route = "data_access_requests_sent", user_id = Empty)
)]
pub(crate) async fn handle_sent(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<SentBody>,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let now = Utc::now();
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .mark_sent(id, body.sent_on, now)
        .await?;
    Ok(view(StatusCode::OK, request, now))
}

/// `POST /api/user/data-access-requests/{id}/extension` — a GDPR controller
/// notified a later date.
///
/// # Errors
/// As [`handle_get`]; a conflict unless it is sent; invalid input for a
/// Quebec request or a date outside the two further months.
#[tracing::instrument(
    skip(resources, headers, body),
    fields(route = "data_access_requests_extension", user_id = Empty)
)]
pub(crate) async fn handle_extension(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<ExtensionBody>,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let now = Utc::now();
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .record_extension(id, body.extended_until, now)
        .await?;
    Ok(view(StatusCode::OK, request, now))
}

/// `POST /api/user/data-access-requests/{id}/answered` — the company answered.
///
/// # Errors
/// As [`handle_get`]; a conflict unless it is sent; invalid input for a date
/// before it was sent or in the future.
#[tracing::instrument(
    skip(resources, headers, body),
    fields(route = "data_access_requests_answered", user_id = Empty)
)]
pub(crate) async fn handle_answered(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<AnsweredBody>,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let now = Utc::now();
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .mark_answered(id, body.answered_on, now)
        .await?;
    Ok(view(StatusCode::OK, request, now))
}

/// `POST /api/user/data-access-requests/{id}/close` — the athlete gave up, or
/// it no longer matters.
///
/// # Errors
/// As [`handle_get`]; a conflict for one answered or closed already.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "data_access_requests_close", user_id = Empty)
)]
pub(crate) async fn handle_close(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let now = Utc::now();
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .close(id, now)
        .await?;
    Ok(view(StatusCode::OK, request, now))
}

/// `GET /api/user/data-access-requests/{id}/letter?locale=` — the letter, in
/// `locale` or the request's own language.
///
/// # Errors
/// As [`handle_get`]; invalid input for a language the platform does not
/// write.
#[tracing::instrument(
    skip(resources, headers, query),
    fields(route = "data_access_requests_letter", user_id = Empty)
)]
pub(crate) async fn handle_letter(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    Query(query): Query<LetterQuery>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = session(&resources, &headers).await?;
    let request = DataAccessRequests::new(&resources.repos, tenant_id, user_id)
        .get(id)
        .await?;
    let user = resources
        .repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    let name = user
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&user.email);
    let locale = query
        .locale
        .unwrap_or_else(|| request.letter_locale.clone());
    let letter = render_letter(
        &resources.messaging_strings,
        &request,
        &locale,
        name,
        Utc::now().date_naive(),
    )?;
    Ok(Json(json!({
        "locale": letter.locale,
        "subject": letter.subject,
        "body": letter.body,
        "text": letter.text(),
        "filename": format!("{}.txt", request.reference),
        "mailto": letter.mailto(&request.controller_contact),
    }))
    .into_response())
}
