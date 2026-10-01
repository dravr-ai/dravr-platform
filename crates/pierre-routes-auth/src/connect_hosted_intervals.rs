// ABOUTME: Hosted Intervals.icu API-key form reached from the chat connect picker, authed by the connect link-token
// ABOUTME: GET renders athlete id + API key fields; POST validates them live, stores them, and redirects to success
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-initiated hosted **Intervals.icu connect form**.
//!
//! Intervals.icu authenticates with an athlete id + personal API key, not
//! OAuth, so the connect picker cannot hand it to `oauth-init`. Its card opens
//! this form instead:
//!
//! 1. `GET /providers/connect/intervals_icu?token=...` verifies the same
//!    connect-scoped link-token the picker does (no nonce burn — the token
//!    stays usable within its short TTL, so a mistyped key can be retried) and
//!    renders the two fields, with the token in a hidden field.
//! 2. `POST /providers/connect/intervals_icu` (form-encoded `token`,
//!    `athlete_id`, `api_key`) verifies the token again, then links the account
//!    through [`link_intervals_icu_account`] — the same validation and
//!    encrypted per-tenant storage the web/mobile Settings form reaches through
//!    `POST /api/providers/intervals_icu/link-credentials`. On success it
//!    303-redirects to the shared connect success page; on a rejected pair it
//!    re-renders the form with the reason, keeping the athlete id and never the
//!    key.
//!
//! Identity (`user_id` + `tenant_id`) comes from the signed token, never from
//! the form. The token is also what makes the POST unforgeable cross-site: an
//! HTML form cannot send the `X-CSRF-Token` header, so the path is listed in
//! `pierre_middleware::csrf`'s exemptions, as the OAuth consent form is.

use std::fmt;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::AppError;
use pierre_middleware::provider_link_token::ProviderLinkTokenClaims;
use serde::Deserialize;
use tracing::{error, info, warn};
use urlencoding::encode;
use uuid::Uuid;

use crate::connect_hosted::{validate_connect_token, ConnectPageQuery};
use crate::connect_hosted_templates;
use crate::intervals_icu::link_intervals_icu_account;
use crate::AuthRoutesContext;

/// Shown when the page is opened or posted without a token.
const MISSING_TOKEN_MESSAGE: &str =
    "Missing connect token. Please request a fresh link from your chat.";

/// Shown when the token does not verify as a connect token.
const INVALID_TOKEN_MESSAGE: &str =
    "This connect link is invalid or has expired. Please request a fresh link from your chat.";

/// Shown when the token verifies but its identity claims do not parse.
const MALFORMED_TOKEN_MESSAGE: &str =
    "This connect link is malformed. Please request a fresh link.";

/// Shown when the link could not be stored for a reason of the server's own.
const SERVER_FAULT_MESSAGE: &str =
    "We could not save your Intervals.icu connection. Please try again in a moment.";

/// The form the hosted Intervals.icu page posts.
#[derive(Deserialize)]
pub struct IntervalsIcuConnectForm {
    /// The signed connect link-token the page was opened with.
    #[serde(default)]
    pub token: String,
    /// Athlete id (e.g. `i123456`).
    #[serde(default)]
    pub athlete_id: String,
    /// Personal API key — secret, never logged nor rendered back.
    #[serde(default)]
    pub api_key: String,
}

impl fmt::Debug for IntervalsIcuConnectForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntervalsIcuConnectForm")
            .field("athlete_id", &self.athlete_id)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// The user and tenant a verified connect token names.
struct ConnectIdentity {
    user_id: Uuid,
    tenant_id: Uuid,
    channel: String,
}

/// Verify a connect link-token and read its identity, or the message the
/// athlete is shown instead.
fn connect_identity(
    resources: &AuthRoutesContext,
    token: &str,
) -> Result<ConnectIdentity, &'static str> {
    if token.is_empty() {
        return Err(MISSING_TOKEN_MESSAGE);
    }
    let claims: ProviderLinkTokenClaims =
        validate_connect_token(resources, token).map_err(|e| {
            warn!(error = %e, "Rejected hosted Intervals.icu form: invalid link-token");
            INVALID_TOKEN_MESSAGE
        })?;
    let (Ok(user_id), Ok(tenant_id)) = (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.tid))
    else {
        return Err(MALFORMED_TOKEN_MESSAGE);
    };
    Ok(ConnectIdentity {
        user_id,
        tenant_id,
        channel: claims.channel,
    })
}

/// GET `/providers/connect/intervals_icu?token=...` — the API-key form.
pub async fn handle_intervals_icu_form_page(
    State(resources): State<AuthRoutesContext>,
    Query(query): Query<ConnectPageQuery>,
) -> Response {
    let token = query.token.unwrap_or_default();
    match connect_identity(&resources, &token) {
        Ok(identity) => {
            info!(
                user_id = %identity.user_id,
                channel = %identity.channel,
                "Rendered hosted Intervals.icu connect form"
            );
            Html(connect_hosted_templates::render_intervals_icu_form(
                &token,
                &identity.channel,
                "",
                "",
            ))
            .into_response()
        }
        Err(message) => {
            Html(connect_hosted_templates::render_connect_error_page(message)).into_response()
        }
    }
}

/// POST `/providers/connect/intervals_icu` — validate, store, and redirect.
pub async fn handle_intervals_icu_form_submit(
    State(resources): State<AuthRoutesContext>,
    Form(form): Form<IntervalsIcuConnectForm>,
) -> Response {
    let identity = match connect_identity(&resources, &form.token) {
        Ok(identity) => identity,
        Err(message) => {
            return (
                StatusCode::UNAUTHORIZED,
                Html(connect_hosted_templates::render_connect_error_page(message)),
            )
                .into_response();
        }
    };

    match link_intervals_icu_account(
        &resources,
        identity.user_id,
        identity.tenant_id,
        &form.athlete_id,
        &form.api_key,
    )
    .await
    {
        Ok(_) => {
            info!(
                user_id = %identity.user_id,
                channel = %identity.channel,
                "Hosted connect: Intervals.icu linked"
            );
            let location = format!(
                "/providers/connect/success?channel={}&target={}",
                encode(&identity.channel),
                INTERVALS_ICU
            );
            (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
        }
        Err(e) => refused_form(&form, &identity, &e),
    }
}

/// The form again, with the reason a link attempt failed and the athlete id
/// refilled — never the key — at the status the failure carries.
fn refused_form(
    form: &IntervalsIcuConnectForm,
    identity: &ConnectIdentity,
    e: &AppError,
) -> Response {
    let message = if e.is_server_fault() {
        error!(user_id = %identity.user_id, error = %e, "Hosted connect: Intervals.icu link failed");
        SERVER_FAULT_MESSAGE.to_owned()
    } else {
        warn!(user_id = %identity.user_id, error = %e, "Hosted connect: Intervals.icu credentials refused");
        e.sanitized_message()
    };
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    (
        status,
        Html(connect_hosted_templates::render_intervals_icu_form(
            &form.token,
            &identity.channel,
            form.athlete_id.trim(),
            &message,
        )),
    )
        .into_response()
}
