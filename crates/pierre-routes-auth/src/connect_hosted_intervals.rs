// ABOUTME: Hosted Intervals.icu page reached from the chat connect picker, authed by the connect link-token
// ABOUTME: GET offers the authorization and the API-key form; POST validates a key live, stores it, and redirects
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-initiated hosted **Intervals.icu connect page**.
//!
//! Intervals.icu links two ways: an athlete authorizes Dravr (OAuth), and a
//! coach pastes their athlete id + personal API key, the only credential that
//! reads the athletes they coach. The picker's card opens this page, which
//! offers both:
//!
//! 1. `GET /providers/connect/intervals_icu?token=...` verifies the same
//!    connect-scoped link-token the picker does (no nonce burn — the token
//!    stays usable within its short TTL, so a mistyped key can be retried) and
//!    renders the page: the authorization first, a link to the picker's
//!    link-token-authed `oauth-init`, while a client resolves for this
//!    athlete (`authorizing_client_resolves`); then the two key fields, with
//!    the token in a hidden field. With no client the key form is all the
//!    page offers.
//! 2. `POST /providers/connect/intervals_icu` (form-encoded `token`,
//!    `athlete_id`, `api_key`) verifies the token again, then links the account
//!    through [`link_intervals_icu_account`] — the same validation and
//!    encrypted per-tenant storage the web/mobile Settings form reaches through
//!    `POST /api/providers/intervals_icu/link-credentials`. On success it
//!    303-redirects to the shared connect success page; on a rejected pair it
//!    re-renders the form with the reason, keeping the athlete id and never the
//!    key.
//!
//! The form, its labels and every reason it shows are written in the locale
//! of the athlete the token names, from the dravr-contremaitre catalogue —
//! the same keys the web and mobile Intervals.icu dialog reads for the field
//! names and for where to find them.
//!
//! Identity (`user_id` + `tenant_id`) comes from the signed token, never from
//! the form. The token is also what makes the POST unforgeable cross-site: an
//! HTML form cannot send the `X-CSRF-Token` header, so the path is listed in
//! `pierre_middleware::csrf`'s exemptions, as the OAuth consent form is.

use secrecy::ExposeSecret;
use std::fmt;

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use pierre_contremaitre::hosted_strings::{
    KEY_HOSTED_ERROR_INVALID_LINK, KEY_HOSTED_ERROR_MALFORMED_LINK, KEY_HOSTED_ERROR_MISSING_TOKEN,
    KEY_HOSTED_INTERVALS_API_KEY_REQUIRED, KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED,
    KEY_HOSTED_INTERVALS_REJECTED, KEY_HOSTED_INTERVALS_SAVE_FAILED,
    KEY_HOSTED_INTERVALS_TOMBSTONED, KEY_HOSTED_INTERVALS_UNAVAILABLE,
};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::AppError;
use pierre_middleware::provider_link_token::ProviderLinkTokenClaims;
use serde::Deserialize;
use tracing::{error, info, warn};
use urlencoding::encode;
use uuid::Uuid;

use crate::connect_hosted::{validate_connect_token, ConnectPageQuery};
use crate::connect_hosted_templates;
use crate::hosted_page::{locale_for_reader, PageStrings};
use crate::intervals_icu::{link_intervals_icu_account, IntervalsLinkError};
use crate::oauth::authorizing_client_resolves;
use crate::short_link::preferred_locale;
use crate::AuthRoutesContext;

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

impl ConnectIdentity {
    /// The locale this athlete reads on the channel the link was sent to.
    async fn locale(&self, resources: &AuthRoutesContext) -> String {
        locale_for_reader(resources, self.user_id, Some(self.tenant_id), &self.channel).await
    }

    /// Whether the page offers this athlete the Intervals.icu authorization:
    /// an OAuth start would find a client to run under. A failed read offers
    /// only the key form, which needs no client.
    async fn offers_authorize(&self, resources: &AuthRoutesContext) -> bool {
        authorizing_client_resolves(resources, self.user_id, self.tenant_id, INTERVALS_ICU)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    user_id = %self.user_id,
                    error = %e,
                    "Hosted Intervals.icu page: could not resolve the OAuth client, offering the API key only"
                );
                false
            })
    }
}

/// Verify a connect link-token and read its identity, or the catalogue key
/// of the message the athlete is shown instead.
fn connect_identity(
    resources: &AuthRoutesContext,
    token: &str,
) -> Result<ConnectIdentity, &'static str> {
    if token.is_empty() {
        return Err(KEY_HOSTED_ERROR_MISSING_TOKEN);
    }
    let claims: ProviderLinkTokenClaims =
        validate_connect_token(resources, token).map_err(|e| {
            warn!(error = %e, "Rejected hosted Intervals.icu form: invalid link-token");
            KEY_HOSTED_ERROR_INVALID_LINK
        })?;
    let (Ok(user_id), Ok(tenant_id)) = (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.tid))
    else {
        return Err(KEY_HOSTED_ERROR_MALFORMED_LINK);
    };
    Ok(ConnectIdentity {
        user_id,
        tenant_id,
        channel: claims.channel,
    })
}

/// The link-error page for a token that names no user, in the browser's
/// language.
fn link_error_page(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
    message_key: &str,
) -> String {
    let strings = PageStrings::new(&resources.messaging_strings, preferred_locale(headers));
    connect_hosted_templates::render_connect_error_page(&strings, message_key)
}

/// GET `/providers/connect/intervals_icu?token=...` — the authorization and
/// the API-key form.
pub async fn handle_intervals_icu_form_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<ConnectPageQuery>,
) -> Response {
    let token = query.token.unwrap_or_default();
    match connect_identity(&resources, token.expose_secret()) {
        Ok(identity) => {
            let locale = identity.locale(&resources).await;
            let offers_authorize = identity.offers_authorize(&resources).await;
            info!(
                user_id = %identity.user_id,
                channel = %identity.channel,
                locale = %locale,
                offers_authorize,
                "Rendered hosted Intervals.icu connect form"
            );
            let strings = PageStrings::new(&resources.messaging_strings, locale);
            Html(connect_hosted_templates::render_intervals_icu_form(
                &strings,
                token.expose_secret(),
                &identity.channel,
                offers_authorize,
                "",
                "",
            ))
            .into_response()
        }
        Err(message_key) => {
            Html(link_error_page(&resources, &headers, message_key)).into_response()
        }
    }
}

/// POST `/providers/connect/intervals_icu` — validate, store, and redirect.
pub async fn handle_intervals_icu_form_submit(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Form(form): Form<IntervalsIcuConnectForm>,
) -> Response {
    let identity = match connect_identity(&resources, &form.token) {
        Ok(identity) => identity,
        Err(message_key) => {
            return (
                StatusCode::UNAUTHORIZED,
                Html(link_error_page(&resources, &headers, message_key)),
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
            // The token rides along so the success page is written in the
            // athlete's locale too.
            let location = format!(
                "/providers/connect/success?channel={}&target={}&token={}",
                encode(&identity.channel),
                INTERVALS_ICU,
                encode(&form.token)
            );
            (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
        }
        Err(e) => refused_form(&resources, &form, &identity, e).await,
    }
}

/// The catalogue key wording why a link attempt failed, and the error the
/// response takes its status from.
fn refusal(e: IntervalsLinkError) -> (&'static str, AppError) {
    let key = match &e {
        IntervalsLinkError::MissingAthleteId => KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED,
        IntervalsLinkError::MissingApiKey => KEY_HOSTED_INTERVALS_API_KEY_REQUIRED,
        IntervalsLinkError::Unavailable(_) => KEY_HOSTED_INTERVALS_UNAVAILABLE,
        // The account belonged to a deleted Dravr account: clearing that is
        // a signed-in choice the app offers, not one this link-token page can
        // make, so the page sends the athlete there.
        IntervalsLinkError::Tombstoned(_) => KEY_HOSTED_INTERVALS_TOMBSTONED,
        // A store that failed is the server's own fault; credentials the
        // provider would not take are the athlete's to fix, as a rejection is.
        IntervalsLinkError::Storage(error) if error.is_server_fault() => {
            KEY_HOSTED_INTERVALS_SAVE_FAILED
        }
        IntervalsLinkError::Rejected(_) | IntervalsLinkError::Storage(_) => {
            KEY_HOSTED_INTERVALS_REJECTED
        }
    };
    (key, e.into())
}

/// The form again, in the athlete's locale, with the reason a link attempt
/// failed and the athlete id refilled — never the key — at the status the
/// failure carries.
async fn refused_form(
    resources: &AuthRoutesContext,
    form: &IntervalsIcuConnectForm,
    identity: &ConnectIdentity,
    e: IntervalsLinkError,
) -> Response {
    let (message_key, e) = refusal(e);
    if e.is_server_fault() {
        error!(user_id = %identity.user_id, error = %e, "Hosted connect: Intervals.icu link failed");
    } else {
        warn!(user_id = %identity.user_id, error = %e, "Hosted connect: Intervals.icu credentials refused");
    }
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    let locale = identity.locale(resources).await;
    let offers_authorize = identity.offers_authorize(resources).await;
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    (
        status,
        Html(connect_hosted_templates::render_intervals_icu_form(
            &strings,
            &form.token,
            &identity.channel,
            offers_authorize,
            form.athlete_id.trim(),
            &strings.get(message_key),
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::errors::ErrorCode;

    #[test]
    fn a_tombstoned_account_is_worded_as_one_to_start_fresh_with_in_the_app() {
        let (key, error) = refusal(IntervalsLinkError::Tombstoned(AppError::new(
            ErrorCode::ResourceLocked,
            "tombstoned",
        )));
        assert_eq!(key, KEY_HOSTED_INTERVALS_TOMBSTONED);
        assert_eq!(error.http_status(), 409);
    }

    #[test]
    fn an_unavailable_provider_keeps_the_unavailable_wording() {
        let (key, _) = refusal(IntervalsLinkError::Unavailable("no client".to_owned()));
        assert_eq!(key, KEY_HOSTED_INTERVALS_UNAVAILABLE);
    }
}
