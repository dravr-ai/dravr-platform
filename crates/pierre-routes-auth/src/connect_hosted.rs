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
//! 2. Renders the three provider cards, computed from the SAME
//!    [`crate::oauth::compute_providers_status`] the web/mobile onboarding uses,
//!    so Strava is OAuth-first while shared-app seats remain and falls back to
//!    the Sciotte credential flow once the cap is reached.
//! 3. Routes the user's choice to either the Sciotte credential state machine
//!    (`/api/providers/sciotte/*`) or the link-token-authed OAuth init
//!    (`/api/providers/connect/oauth-init/{provider}`).
//!
//! Passwords and OAuth consent happen here over TLS — never in the chat
//! transcript. Identity (`user_id` + `tenant_id`) comes from the signed token,
//! so the resulting connection is stored under the user's own tenant.

use crate::sciotte_hosted_templates::{exposure_notice, ExposureNotice};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use urlencoding::encode;

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
use crate::oauth::{compute_providers_status, require_oauth_start_notice};
use crate::AuthRoutesContext;

/// Bare OAuth provider rows that the connect picker hides — Strava is the
/// `sciotte` card, Garmin the `sciotte_garmin` card and TrainingPeaks the
/// `sciotte_trainingpeaks` card; the API-key and
/// synthetic providers are out of the messaging connect scope.
const HIDDEN_FROM_PICKER: &[&str] = &[
    "strava",
    "garmin",
    "synthetic",
    "synthetic_sleep",
    "intervals_icu",
];

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
}

/// One selectable card on the hosted picker. Serialized into the page so its JS
/// can route a click to the OAuth or Sciotte sub-flow.
#[derive(Debug, Serialize)]
struct ConnectProviderCard {
    /// Provider id passed to `oauth-init` (OAuth kind) or used for routing.
    provider: String,
    /// User-facing label (e.g. "Strava", "Garmin", "Whoop").
    display_name: String,
    /// Already connected — the card is shown disabled.
    connected: bool,
    /// "oauth" (full-page redirect to consent) or "sciotte" (credential form).
    kind: &'static str,
    /// Sciotte target ("strava" / "garmin" / "trainingpeaks" / "coros"); empty for OAuth cards.
    target: String,
    /// The page must show the provider's notice, with a required checkbox,
    /// before the credentials form (TrainingPeaks and COROS) or before the
    /// OAuth redirect (WHOOP), until accepted.
    consent_required: bool,
    /// The provider's notice, which the page fills its notice block with when
    /// the card is picked. Absent for a provider with none.
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<ExposureNotice>,
    /// What the provider's own login asks for: `"username"` (TrainingPeaks)
    /// or `"email"`. Read by the page for Sciotte cards only.
    login_identifier: &'static str,
}

/// Verify a connect-scoped link-token. A narrow per-provider token (e.g. a
/// Canot-minted `provider:sciotte:login`) is rejected here — only a connect
/// token may drive the picker and the OAuth-init endpoint.
fn validate_connect_token(
    resources: &AuthRoutesContext,
    token: &str,
) -> Result<ProviderLinkTokenClaims, AppError> {
    verify_link_token(token, &resources.admin_jwt_secret, CONNECT_PROVIDER)
}

/// Map the provider catalogue into the picker's cards, applying the web
/// onboarding OAuth-first decision tree.
async fn build_connect_providers(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
) -> Vec<ConnectProviderCard> {
    let status = compute_providers_status(resources, user_id, tenant_id).await;

    // Mirror ProviderConnectionCards: the raw `strava` OAuth row is hidden (the
    // sciotte card IS the Strava data path), but its connected state must merge
    // into that card — otherwise a user already connected via Strava OAuth sees
    // "Strava — Authorize" and can be pushed through a needless re-consent.
    let strava_oauth_connected = status
        .providers
        .iter()
        .any(|p| p.provider == "strava" && p.connected);

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
                    connected: p.connected || strava_oauth_connected,
                    kind: if oauth_first { "oauth" } else { "sciotte" },
                    target: "strava".to_owned(),
                    consent_required: p.consent_required,
                    notice: exposure_notice("strava").cloned(),
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
                        connected: p.connected,
                        kind: "sciotte",
                        target: target.to_owned(),
                        consent_required: p.consent_required,
                        notice: exposure_notice(target).cloned(),
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
            // Any remaining OAuth provider (Whoop, and future keepers).
            _ if p.requires_oauth => cards.push(ConnectProviderCard {
                provider: p.provider,
                display_name: p.display_name,
                connected: p.connected,
                kind: "oauth",
                target: String::new(),
                consent_required: p.consent_required,
                notice: exposure_notice(&provider_name).cloned(),
                login_identifier: "email",
            }),
            // Non-OAuth, non-Sciotte (e.g. synthetic) is not offered in chat.
            _ => {}
        }
    }
    cards
}

/// GET `/providers/connect?token=...` — the hosted provider picker.
pub async fn handle_connect_hosted_page(
    State(resources): State<AuthRoutesContext>,
    Query(query): Query<ConnectPageQuery>,
) -> Response {
    let Some(token) = query.token.as_deref() else {
        return Html(connect_hosted_templates::render_connect_error_page(
            "Missing connect token. Please request a fresh link from your chat.",
        ))
        .into_response();
    };

    let claims = match validate_connect_token(&resources, token) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Rejected hosted connect page: invalid link-token");
            return Html(connect_hosted_templates::render_connect_error_page(
                "This connect link is invalid or has expired. Please request a fresh link from your chat.",
            ))
            .into_response();
        }
    };

    let Ok(user_id) = Uuid::parse_str(&claims.sub) else {
        return Html(connect_hosted_templates::render_connect_error_page(
            "This connect link is malformed. Please request a fresh link.",
        ))
        .into_response();
    };

    // The link-token names the session's tenant; one it cannot parse asks
    // for no notice, as a session without a tenant does.
    let tenant_id = Uuid::parse_str(&claims.tid).ok();
    let cards = build_connect_providers(&resources, user_id, tenant_id).await;
    let providers_json = serde_json::to_string(&cards).unwrap_or_else(|_| "[]".to_owned());

    info!(
        user_id = %claims.sub,
        channel = %claims.channel,
        providers = cards.len(),
        "Rendered hosted connect picker"
    );

    Html(connect_hosted_templates::render_connect_page(
        token,
        &claims.channel,
        &providers_json,
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
        return Html(connect_hosted_templates::render_connect_error_page(
            "Connecting this provider needs its notice accepted first. Please go back, tick the notice and try again.",
        ))
        .into_response();
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
            error!(provider = %provider, user_id = %user_id, error = %e, "Hosted connect: OAuth init failed");
            Html(connect_hosted_templates::render_connect_error_page(
                "We couldn't start the connection for this provider. Please go back and try again.",
            ))
            .into_response()
        }
    }
}

/// GET `/providers/connect/success` — shown after a successful connection.
pub async fn handle_connect_hosted_success_page(
    Query(query): Query<ConnectSuccessQuery>,
) -> Response {
    let channel = query.channel.as_deref().unwrap_or("");
    let target = query.target.as_deref().unwrap_or("");
    Html(connect_hosted_templates::render_connect_success_page(
        channel, target,
    ))
    .into_response()
}
