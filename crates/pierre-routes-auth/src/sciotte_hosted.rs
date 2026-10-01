// ABOUTME: Hosted Sciotte login page for channel-initiated provider connection
// ABOUTME: Serves an HTML page that runs the full Sciotte login state machine
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Hosted Sciotte login for channel-initiated flows.
//!
//! This module implements the three public-facing pages the user sees when a
//! chat channel (Slack, Discord, Telegram, ...) hands them a login URL:
//!
//! - `GET /providers/sciotte/login?token=...` — renders the login page. The token
//!   query parameter is a provider link-token the messaging pipeline or the
//!   backfill notifier minted in-process. The page embeds the token
//!   so its client-side JS can call the existing `/api/providers/sciotte/*` API
//!   with `Authorization: Bearer <token>`.
//! - `GET /providers/sciotte/success` — confirmation page, auto-closes and can
//!   deep-link back to the originating channel.
//! - `GET /providers/sciotte/error?message=...` — renders a user-facing error page.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use pierre_contremaitre::hosted_strings::{
    KEY_HOSTED_ERROR_GENERIC, KEY_HOSTED_ERROR_INVALID_LINK, KEY_HOSTED_ERROR_LINK_ALREADY_OPENED,
    KEY_HOSTED_ERROR_MISSING_TOKEN,
};
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::hosted_page::{locale_for_claims, locale_for_token, PageStrings};
use crate::sciotte_hosted_templates;
use crate::short_link::preferred_locale;
use crate::AuthRoutesContext;
use pierre_middleware::provider_link_token::{verify_link_token, ProviderLinkTokenClaims};
use pierre_providers::backend_resolver;
use pierre_providers::sciotte_provider::SciotteTarget;
use pierre_services::provider_notice::asks_for_notice;

/// Default target platform when the caller does not specify one
const DEFAULT_TARGET: &str = "strava";

/// The provider a hosted-login link-token is minted for.
const LOGIN_TOKEN_PROVIDER: &str = "sciotte";

/// Query parameters for the hosted login page
#[derive(Debug, Deserialize)]
pub struct HostedLoginQuery {
    /// The signed link-token that authorizes this page to call
    /// `/api/providers/sciotte/*` on behalf of a user
    pub token: Option<String>,
}

/// Query parameters for the hosted error page
#[derive(Debug, Deserialize)]
pub struct HostedErrorQuery {
    /// User-facing error message
    pub message: Option<String>,
}

/// Query parameters for the hosted success page
#[derive(Debug, Deserialize)]
pub struct HostedSuccessQuery {
    /// Optional channel slug so the success page can show channel-specific
    /// follow-up text (e.g. "Return to Slack")
    pub channel: Option<String>,
    /// Target platform shown on the success page ("strava" / "garmin")
    pub target: Option<String>,
    /// The link-token the login page was opened with, which names the athlete
    /// whose locale the page is written in. Absent or expired, the page
    /// follows the browser's `Accept-Language`.
    pub token: Option<String>,
}

/// The locale of the athlete `claims` names, or the browser's when the
/// subject is not a user id.
async fn claims_locale(
    resources: &AuthRoutesContext,
    claims: &ProviderLinkTokenClaims,
    headers: &HeaderMap,
) -> String {
    locale_for_claims(resources, claims)
        .await
        .unwrap_or_else(|| preferred_locale(headers).to_owned())
}

/// Validate the link-token and burn its nonce, returning the verified claims.
///
/// The rejection is the rendered error page, boxed so the `Err` side stays small.
async fn validate_and_burn_link_token(
    resources: &AuthRoutesContext,
    token: &str,
    headers: &HeaderMap,
) -> Result<ProviderLinkTokenClaims, Box<Response>> {
    let claims = verify_link_token(token, &resources.admin_jwt_secret, LOGIN_TOKEN_PROVIDER)
        .map_err(|e| {
            warn!(error = %e, "Rejected hosted-login page: invalid link-token");
            // No verified user to ask: the browser's language.
            Box::new(render_error_response(
                resources,
                preferred_locale(headers),
                KEY_HOSTED_ERROR_INVALID_LINK,
            ))
        })?;

    // SECURITY: Burn the jti on the first page load so the URL can't be replayed.
    // The token remains valid for the login, 2FA and OTP POSTs within its TTL
    // (`PROVIDER_LINK_TOKEN_TTL_MINUTES`) so the multi-step Sciotte flow
    // (credentials -> 2FA -> OTP) can complete.
    if let Err(e) = resources.nonce_store.burn(&claims.jti).await {
        warn!(jti = %claims.jti, error = %e, "Hosted-login page: link already consumed");
        let locale = claims_locale(resources, &claims, headers).await;
        return Err(Box::new(render_error_response(
            resources,
            &locale,
            KEY_HOSTED_ERROR_LINK_ALREADY_OPENED,
        )));
    }

    Ok(claims)
}

/// GET `/providers/sciotte/login?token=...`
///
/// Renders the hosted Sciotte login page in the locale of the athlete the
/// link-token names. Validates the link-token up-front so a user with an
/// invalid/expired link sees the error page instead of a broken-looking form.
pub async fn handle_sciotte_hosted_login_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<HostedLoginQuery>,
) -> Response {
    let Some(token) = query.token.as_deref() else {
        return render_error_response(
            &resources,
            preferred_locale(&headers),
            KEY_HOSTED_ERROR_MISSING_TOKEN,
        );
    };

    let claims = match validate_and_burn_link_token(&resources, token, &headers).await {
        Ok(c) => c,
        Err(resp) => return *resp,
    };

    // Target is clamped server-side at mint time; re-validate on render in case of tampering.
    let target = backend_resolver::mirror_backend_for(&claims.tgt)
        .and_then(backend_resolver::hosted_login_target)
        .unwrap_or(DEFAULT_TARGET);

    let locale = claims_locale(&resources, &claims, &headers).await;
    info!(
        user_id = %claims.sub,
        channel = %claims.channel,
        target = %target,
        locale = %locale,
        "Rendered Sciotte hosted-login page"
    );

    // A provider whose exposure notice is in force for this account asks for
    // it until the account has accepted its current version. An unreadable
    // acceptance asks again: the login refuses without it. Ids the token
    // cannot name leave the page to the login, which re-checks with its own.
    let backend = SciotteTarget::from_target_param(target).provider_name();
    let consent_required = match (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.tid)) {
        (Ok(user_id), Ok(tenant_id)) => {
            asks_for_notice(&resources.repos, tenant_id, user_id, backend).await
        }
        _ => false,
    };

    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(sciotte_hosted_templates::render_login_page(
        &strings,
        token,
        target,
        &claims.channel,
        consent_required,
    ))
    .into_response()
}

/// GET `/providers/sciotte/success`
pub async fn handle_sciotte_hosted_success_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<HostedSuccessQuery>,
) -> Response {
    let channel = query.channel.as_deref().unwrap_or("");
    let target = query.target.as_deref().unwrap_or(DEFAULT_TARGET);
    let locale = locale_for_token(
        &resources,
        query.token.as_deref(),
        LOGIN_TOKEN_PROVIDER,
        &headers,
    )
    .await;
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(sciotte_hosted_templates::render_success_page(
        &strings, channel, target,
    ))
    .into_response()
}

/// GET `/providers/sciotte/error?message=...`
///
/// The message is the caller's own text and is shown as given; without one
/// the page words the failure itself, in the browser's language.
pub async fn handle_sciotte_hosted_error_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<HostedErrorQuery>,
) -> Response {
    let strings = PageStrings::new(&resources.messaging_strings, preferred_locale(&headers));
    let message = query
        .message
        .unwrap_or_else(|| strings.get(KEY_HOSTED_ERROR_GENERIC));
    Html(sciotte_hosted_templates::render_error_page(
        &strings, &message,
    ))
    .into_response()
}

/// The link-error page in `locale`, worded by the catalogue's `message_key`.
fn render_error_response(
    resources: &AuthRoutesContext,
    locale: &str,
    message_key: &str,
) -> Response {
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(sciotte_hosted_templates::render_error_page(
        &strings,
        &strings.get(message_key),
    ))
    .into_response()
}
