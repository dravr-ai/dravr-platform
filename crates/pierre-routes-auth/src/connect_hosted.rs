// ABOUTME: Hosted connect picker for channel-initiated provider connection (OAuth + Sciotte)
// ABOUTME: Generalizes the Sciotte hosted-login bridge into a provider picker authed by a connect link-token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-initiated hosted **connect picker**.
//!
//! The platform's own messaging pipeline mints a generic connect link-token
//! ([`pierre_middleware::provider_link_token::mint_connect_link_token`]) and
//! hands the user a `/providers/connect?token=...` URL. This page:
//!
//! 1. Validates the connect-scoped link-token (no nonce burn — the token stays
//!    usable so the user can try one provider, fail, and try another; the
//!    reusable window is capped by the deliberately short
//!    `CONNECT_LINK_TOKEN_TTL_MINUTES`, not by a burn).
//! 2. Renders the provider cards, computed from the SAME
//!    [`crate::oauth::compute_providers_status`] the web/mobile onboarding uses,
//!    so Strava is OAuth-first while shared-app seats remain and falls back to
//!    the Sciotte credential flow once the cap is reached.
//! 3. Routes the user's choice to the Sciotte credential state machine
//!    (`/api/providers/sciotte/*`), the link-token-authed OAuth init
//!    (`/api/providers/connect/oauth-init/{provider}`), or — for Intervals.icu,
//!    which takes an athlete id + API key — the hosted API-key form
//!    (`/providers/connect/intervals_icu`, [`crate::connect_hosted_intervals`]).
//!
//! Every page is written in the locale of the athlete the link-token names
//! ([`crate::hosted_page`]), from the dravr-contremaitre catalogue. A page
//! with no verifiable token follows the browser's `Accept-Language`.
//!
//! Passwords and OAuth consent happen here over TLS — never in the chat
//! transcript. Identity (`user_id` + `tenant_id`) comes from the signed token,
//! so the resulting connection is stored under the user's own tenant.

use std::fmt::Display;

use crate::sciotte_hosted_templates::{exposure_notice, ExposureNotice};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use pierre_contremaitre::hosted_strings::{
    KEY_HOSTED_ERROR_INVALID_LINK, KEY_HOSTED_ERROR_LOAD_FAILED, KEY_HOSTED_ERROR_MALFORMED_LINK,
    KEY_HOSTED_ERROR_MISSING_TOKEN, KEY_HOSTED_ERROR_NOTICE_REQUIRED,
    KEY_HOSTED_ERROR_OAUTH_START_FAILED,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use urlencoding::encode;

use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::provider_link_token::{
    verify_link_token, ProviderLinkTokenClaims, CONNECT_PROVIDER,
};
use pierre_providers::backend_resolver;
use pierre_providers::sciotte_provider::SciotteTarget;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use uuid::Uuid;

use crate::connect_hosted_templates;
use crate::hosted_page::{locale_for_reader, locale_for_token, PageStrings};
use crate::oauth::{compute_providers_status, require_oauth_start_notice};
use crate::short_link::preferred_locale;
use crate::AuthRoutesContext;

/// Catalogue cards the connect picker does not offer: the synthetic providers
/// are out of the messaging connect scope. A raw card a mirror covers
/// (`strava`, `garmin`) is never served by the catalogue.
const HIDDEN_FROM_PICKER: &[&str] = &["synthetic", "synthetic_sleep"];

/// Query parameters for the hosted connect page.
#[derive(Debug, Deserialize)]
pub struct ConnectPageQuery {
    /// The signed connect link-token authorizing this page.
    pub token: Option<String>,
}

/// Query parameters for the hosted OAuth-init endpoint.
#[derive(Debug, Deserialize)]
pub struct ConnectOAuthInitQuery {
    /// The signed connect link-token authorizing the OAuth start.
    pub token: Option<String>,
    /// The athlete ticked the provider's notice on the picker (WHOOP's owner
    /// authorization) before this start.
    #[serde(default)]
    pub tos_consent: bool,
}

/// Query parameters for the connect success page.
#[derive(Debug, Deserialize)]
pub struct ConnectSuccessQuery {
    /// Originating channel slug (for "return to {channel}" copy).
    pub channel: Option<String>,
    /// Provider/target label shown on the success page.
    pub target: Option<String>,
    /// The connect link-token the picker was opened with, which names the
    /// athlete whose locale the page is written in. Absent or expired, the
    /// page follows the browser's `Accept-Language`.
    pub token: Option<String>,
}

/// One selectable card on the hosted picker. Serialized into the page so its JS
/// can route a click to the OAuth or Sciotte sub-flow, or to the card's own
/// hosted form.
#[derive(Debug, Serialize)]
struct ConnectProviderCard {
    /// Provider id passed to `oauth-init` (OAuth kind) or used for routing.
    provider: String,
    /// User-facing label (e.g. "Strava", "Garmin", "Whoop").
    display_name: String,
    /// The line under the label, in the user's locale, as `/api/providers`
    /// serves it.
    description: String,
    /// Already connected — the card is shown disabled.
    connected: bool,
    /// "oauth" (full-page redirect to consent), "sciotte" (credential form) or
    /// "`api_key`" (the hosted Intervals.icu API-key form).
    kind: &'static str,
    /// Sciotte target ("strava" / "garmin" / "trainingpeaks" / "coros"), or
    /// `intervals_icu` for the API-key card; empty for OAuth cards.
    target: String,
    /// The page must show the provider's notice, with a required checkbox,
    /// before the credentials form (TrainingPeaks and COROS) or before the
    /// OAuth redirect (WHOOP), until accepted.
    consent_required: bool,
    /// The provider's notice in the user's locale, which the page fills its
    /// notice block with when the card is picked. Absent for a provider with
    /// none.
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<ExposureNotice>,
    /// Where picking the card navigates, for a card whose flow is a hosted
    /// page of its own (the Intervals.icu API-key form): a same-origin path
    /// carrying the connect link-token. Absent for the OAuth and Sciotte
    /// cards, whose flows the picker page runs itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    form_url: Option<String>,
    /// What the provider's own login asks for: `"username"` (TrainingPeaks)
    /// or `"email"`. Read by the page for Sciotte cards only.
    login_identifier: &'static str,
}

/// Verify a connect-scoped link-token. A narrow per-provider token (e.g. a
/// Canot-minted `provider:sciotte:login`) is rejected here — only a connect
/// token may drive the picker and the OAuth-init endpoint.
pub fn validate_connect_token(
    resources: &AuthRoutesContext,
    token: &str,
) -> Result<ProviderLinkTokenClaims, AppError> {
    verify_link_token(token, &resources.admin_jwt_secret, CONNECT_PROVIDER)
}

/// Map the provider catalogue into the picker's cards, applying the web
/// onboarding OAuth-first decision tree. `strings` words each card's notice
/// and description in the page's locale.
async fn build_connect_providers(
    resources: &AuthRoutesContext,
    strings: &PageStrings<'_>,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    link_token: &str,
) -> Result<Vec<ConnectProviderCard>, AppError> {
    let status = compute_providers_status(resources, user_id, tenant_id, strings.locale()).await?;

    // The `sciotte` card already counts a native Strava OAuth grant as
    // connected (`card_is_connected`), so a user connected either way never
    // sees "Strava — Authorize" and a needless re-consent.
    let mut cards = Vec::new();

    for p in status.providers {
        let provider_name = p.provider.clone();
        if HIDDEN_FROM_PICKER.contains(&provider_name.as_str()) {
            continue;
        }
        match provider_name.as_str() {
            // Strava card: OAuth-first while shared-app seats remain, else the
            // Sciotte scraper. Mirrors ProviderConnectionCards / OnboardingConnectScreen.
            "sciotte" => {
                let oauth_first = p.recommended_backend.as_deref() == Some("oauth");
                cards.push(ConnectProviderCard {
                    provider: if oauth_first {
                        "strava".to_owned()
                    } else {
                        "sciotte".to_owned()
                    },
                    display_name: p.display_name,
                    description: p.description,
                    connected: p.connected,
                    kind: if oauth_first { "oauth" } else { "sciotte" },
                    target: "strava".to_owned(),
                    consent_required: p.consent_required,
                    notice: exposure_notice(strings, "strava"),
                    form_url: None,
                    login_identifier: "email",
                });
            }
            // Garmin, TrainingPeaks and COROS: always the credential/scraper
            // flow — none has an OAuth backend Pierre can call.
            "sciotte_garmin" | "sciotte_trainingpeaks" | "sciotte_coros" => {
                if let Some(target) = backend_resolver::hosted_login_target(&provider_name) {
                    cards.push(ConnectProviderCard {
                        provider: provider_name.clone(),
                        display_name: p.display_name,
                        description: p.description,
                        connected: p.connected,
                        kind: "sciotte",
                        target: target.to_owned(),
                        consent_required: p.consent_required,
                        notice: exposure_notice(strings, target),
                        form_url: None,
                        login_identifier: if SciotteTarget::from_target_param(target)
                            .signs_in_with_username()
                        {
                            "username"
                        } else {
                            "email"
                        },
                    });
                }
            }
            // Intervals.icu: an athlete id + API key, collected by the hosted
            // API-key form rather than an OAuth consent screen.
            INTERVALS_ICU => cards.push(ConnectProviderCard {
                provider: p.provider,
                display_name: p.display_name,
                description: p.description,
                connected: p.connected,
                kind: "api_key",
                target: INTERVALS_ICU.to_owned(),
                consent_required: false,
                notice: None,
                form_url: Some(format!(
                    "/providers/connect/{INTERVALS_ICU}?token={}",
                    encode(link_token)
                )),
                login_identifier: "email",
            }),
            // Any remaining OAuth provider (Whoop, and future keepers).
            _ if p.requires_oauth => cards.push(ConnectProviderCard {
                provider: p.provider,
                display_name: p.display_name,
                description: p.description,
                connected: p.connected,
                kind: "oauth",
                target: String::new(),
                consent_required: p.consent_required,
                notice: exposure_notice(strings, &provider_name),
                form_url: None,
                login_identifier: "email",
            }),
            // Non-OAuth, non-Sciotte (e.g. synthetic) is not offered in chat.
            _ => {}
        }
    }
    Ok(cards)
}

/// The link-error page in `locale`, worded by the catalogue's `message_key`.
fn error_page(resources: &AuthRoutesContext, locale: &str, message_key: &str) -> Response {
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(connect_hosted_templates::render_connect_error_page(
        &strings,
        message_key,
    ))
    .into_response()
}

/// GET `/providers/connect?token=...` — the hosted provider picker.
pub async fn handle_connect_hosted_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<ConnectPageQuery>,
) -> Response {
    // Until the token names a user there is nobody's stored locale to read:
    // the pages that refuse the link follow the browser's language.
    let browser_locale = preferred_locale(&headers);
    let Some(token) = query.token.as_deref() else {
        return error_page(&resources, browser_locale, KEY_HOSTED_ERROR_MISSING_TOKEN);
    };

    let claims = match validate_connect_token(&resources, token) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Rejected hosted connect page: invalid link-token");
            return error_page(&resources, browser_locale, KEY_HOSTED_ERROR_INVALID_LINK);
        }
    };

    let Ok(user_id) = Uuid::parse_str(&claims.sub) else {
        return error_page(&resources, browser_locale, KEY_HOSTED_ERROR_MALFORMED_LINK);
    };

    // The link-token names the session's tenant; one it cannot parse asks
    // for no notice, as a session without a tenant does.
    let tenant_id = Uuid::parse_str(&claims.tid).ok();

    // The page is written in the stored locale of the athlete the token
    // names, on the channel it names — resolved, never read from the request.
    let locale = locale_for_reader(&resources, user_id, tenant_id, &claims.channel).await;
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    let cards = build_connect_providers(&resources, &strings, user_id, tenant_id, token)
        .await
        .and_then(|cards| {
            let json = serde_json::to_value(&cards)
                .map_err(|e| AppError::internal(format!("provider cards are not JSON: {e}")))?;
            Ok((cards.len(), json))
        });
    let (card_count, providers) = match cards {
        Ok(cards) => cards,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Could not read the user's connections for the hosted connect picker");
            return error_page(&resources, strings.locale(), KEY_HOSTED_ERROR_LOAD_FAILED);
        }
    };

    info!(
        user_id = %claims.sub,
        channel = %claims.channel,
        providers = card_count,
        locale = strings.locale(),
        "Rendered hosted connect picker"
    );

    Html(connect_hosted_templates::render_connect_page(
        &strings,
        token,
        &claims.channel,
        &providers,
    ))
    .into_response()
}

/// GET `/api/providers/connect/oauth-init/{provider}?token=...`
///
/// Link-token-authed OAuth initiation: recovers `user_id` + `tenant_id` from the
/// connect token (no session cookie), applies the provider's notice
/// precondition (`&tos_consent=true` carries the acceptance the picker took),
/// mints the authorize URL via the shared [`OAuthService`], and 302-redirects
/// the browser to the provider's consent screen. The existing session-less
/// callback completes the exchange.
pub async fn handle_connect_oauth_init(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    Query(query): Query<ConnectOAuthInitQuery>,
) -> Response {
    let Some(token) = query.token.as_deref() else {
        return AppError::auth_invalid("Missing connect token").into_response();
    };
    let claims = match validate_connect_token(&resources, token) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let (Ok(user_id), Ok(tenant_uuid)) =
        (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.tid))
    else {
        return AppError::auth_invalid("Connect token has malformed identity claims")
            .into_response();
    };
    let tenant_id = TenantId::from_uuid(tenant_uuid);

    // The picker shows the provider's notice before this redirect; a start
    // without its acceptance goes back to the picker rather than on to the
    // provider.
    if let Err(e) =
        require_oauth_start_notice(&resources, user_id, tenant_id, &provider, query.tos_consent)
            .await
    {
        warn!(provider = %provider, user_id = %user_id, error = %e, "Hosted connect: OAuth start refused");
        let locale =
            locale_for_reader(&resources, user_id, Some(tenant_uuid), &claims.channel).await;
        return error_page(&resources, &locale, KEY_HOSTED_ERROR_NOTICE_REQUIRED);
    }

    let oauth_service = OAuthService::new(resources.data.clone(), resources.config.clone());

    // On return from the provider's consent screen, bounce back to THIS hosted
    // picker (carrying the same connect token) rather than the SPA: on success
    // the picker shows its success page, and on failure it opens the Sciotte
    // credential fallback for the same provider — so a channel user is never
    // stranded on the SPA's "Connection Failed" page. The callback validates
    // this URL against the redirect allowlist (same origin as base_url) before
    // honoring it.
    let return_url = format!(
        "{}/providers/connect?token={}",
        resources.config.base_url.trim_end_matches('/'),
        encode(token)
    );

    match oauth_service
        .get_auth_url(
            user_id,
            tenant_id,
            &provider,
            AuthUrlOptions {
                return_redirect: Some(&return_url),
            },
        )
        .await
    {
        Ok(auth_response) => {
            info!(provider = %provider, user_id = %user_id, "Hosted connect: OAuth URL issued");
            (
                StatusCode::FOUND,
                [(header::LOCATION, auth_response.authorization_url)],
            )
                .into_response()
        }
        Err(e) => {
            log_oauth_init_failure(&provider, &user_id, &e);
            let locale =
                locale_for_reader(&resources, user_id, Some(tenant_uuid), &claims.channel).await;
            error_page(&resources, &locale, KEY_HOSTED_ERROR_OAUTH_START_FAILED)
        }
    }
}

/// Log a hosted-connect OAuth start that failed, at the level its cause
/// deserves.
///
/// `{provider}` is the caller's path segment: naming one that does not exist,
/// or has no OAuth, is refused, not a fault.
fn log_oauth_init_failure(provider: &str, user_id: &dyn Display, e: &AppError) {
    if e.is_server_fault() {
        error!(provider = %provider, user_id = %user_id, error = %e, "Hosted connect: OAuth init failed");
    } else {
        warn!(provider = %provider, user_id = %user_id, error = %e, "Hosted connect: OAuth init refused");
    }
}

/// GET `/providers/connect/success` — shown after a successful connection.
///
/// The picker and the Intervals.icu form send the connect token along, so the
/// page is written in the locale of the athlete it names.
pub async fn handle_connect_hosted_success_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<ConnectSuccessQuery>,
) -> Response {
    let channel = query.channel.as_deref().unwrap_or("");
    let target = query.target.as_deref().unwrap_or("");
    let locale = locale_for_token(
        &resources,
        query.token.as_deref(),
        CONNECT_PROVIDER,
        &headers,
    )
    .await;
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(connect_hosted_templates::render_connect_success_page(
        &strings, channel, target,
    ))
    .into_response()
}
