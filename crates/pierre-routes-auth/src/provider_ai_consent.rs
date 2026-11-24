// ABOUTME: Read, give or withdraw the account's consent to AI use of a provider's data, or to sending a provider data, from settings
// ABOUTME: GET, PUT and DELETE /api/providers/{provider}/ai-consent, PUT and DELETE /transfer-consent — withdrawal is one call, as simple as the consent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's withdrawable consents, per provider.
//!
//! Two kinds share one body here:
//!
//! - **AI use** (`/ai-consent`). A provider whose notice is a consent to AI
//!   use (WHOOP's owner authorization, Garmin's AI consent) lets the athlete
//!   withdraw it at any time, as simply as it was given (carnet#726). One
//!   `DELETE` withdraws it: from then on no model reads that provider's data,
//!   while the athlete still sees what is stored and the connection stays
//!   live. `GET` reads it for any key, a consent class included
//!   (`sensitive_t2`, the athlete's health records, J2), which has no card.
//! - **Transfer** (`/transfer-consent`). Garmin's consent to Dravr sending
//!   the athlete's planned workouts to their Garmin Connect calendar
//!   (carnet#1390): while it is not given, Dravr writes nothing to Garmin.
//!
//! One `PUT` gives either again, in the notice's current version. A card's
//! key reaches the consent that covers it: the Garmin card is served as
//! `sciotte_garmin`, and its consents are recorded under `garmin` and
//! `garmin_workout_transfer` ([`consent_key`]). The card's `ai_consent` and
//! `transfer_consent` on `GET /api/providers` read the answers back.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::constants::oauth_providers::{
    consent_key, provider_notice, sensitive_display_name, NoticeKind,
};
use pierre_core::errors::AppError;
use pierre_runtime_context::require_tenant_in;
use pierre_services::provider_notice::{consent_state, grant_consent, withdraw_consent};
use serde_json::json;

use crate::AuthRoutesContext;

/// The response field one kind of consent is read back under.
const fn field(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::TransferConsent => "transfer_consent",
        NoticeKind::AiConsent | NoticeKind::TermsExposure => "ai_consent",
    }
}

/// The key the `kind` consent named by `provider` is recorded under: the
/// consent covering that backend, else `provider` itself (a consent class, or
/// a key that asks for nothing and so reads back `null`).
fn resolved_key(provider: &str, kind: NoticeKind) -> String {
    consent_key(provider, kind).map_or_else(|| provider.to_owned(), str::to_owned)
}

/// The provider as a refusal names it: a consent class's fixed display name,
/// else the registered display name of the provider the consent is about,
/// else the key.
fn brand(resources: &AuthRoutesContext, key: &str) -> String {
    if let Some(name) = sensitive_display_name(key) {
        return name.to_owned();
    }
    let about = provider_notice(key)
        .and_then(|notice| notice.covers.first().copied())
        .unwrap_or(key);
    resources
        .provider_registry
        .get_descriptor(about)
        .map_or_else(|| key.to_owned(), |d| d.display_name().to_owned())
}

/// Read the account's `kind` consent for `provider`.
async fn read_consent(
    resources: &AuthRoutesContext,
    provider: &str,
    headers: &HeaderMap,
    kind: NoticeKind,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    let tenant_id = require_tenant_in(resources.repos.tenants.as_ref(), &auth)
        .await?
        .as_uuid();
    let key = resolved_key(provider, kind);
    let state = consent_state(&resources.repos, tenant_id, auth.user_id, &key, kind).await;
    Ok((
        StatusCode::OK,
        Json(json!({ "provider": provider, field(kind): state })),
    )
        .into_response())
}

/// Give the account's `kind` consent for `provider`.
async fn give_consent(
    resources: &AuthRoutesContext,
    provider: &str,
    headers: &HeaderMap,
    kind: NoticeKind,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    // A consent is the athlete's to give; an admin's support session may
    // withdraw one, never give it (carnet#1634).
    auth.refuse_impersonation(
        resources.repos.admin_audit.as_ref(),
        match kind {
            NoticeKind::TransferConsent => "give consent to send a provider the athlete's data",
            NoticeKind::AiConsent | NoticeKind::TermsExposure => {
                "give consent to AI use of a provider's data"
            }
        },
    )
    .await?;
    let tenant_id = require_tenant_in(resources.repos.tenants.as_ref(), &auth)
        .await?
        .as_uuid();
    let key = resolved_key(provider, kind);
    let brand = brand(resources, &key);
    grant_consent(
        &resources.repos,
        tenant_id,
        auth.user_id,
        &key,
        kind,
        &brand,
    )
    .await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "provider": provider, field(kind): true })),
    )
        .into_response())
}

/// Withdraw the account's `kind` consent for `provider`.
async fn take_back_consent(
    resources: &AuthRoutesContext,
    provider: &str,
    headers: &HeaderMap,
    kind: NoticeKind,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    let key = resolved_key(provider, kind);
    let brand = brand(resources, &key);
    withdraw_consent(&resources.repos, auth.user_id, &key, kind, &brand).await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "provider": provider, field(kind): false })),
    )
        .into_response())
}

/// Read the account's consent to AI use of `provider`'s data.
///
/// `GET /api/providers/{provider}/ai-consent` — `ai_consent` is `true` while
/// it is given in the notice's current version, `false` while it is not, and
/// `null` for a key that asks no such consent of this account.
///
/// # Errors
/// Unauthenticated; or an account with no tenant.
pub(crate) async fn handle_get_ai_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    read_consent(&resources, &provider, &headers, NoticeKind::AiConsent).await
}

/// Give the account's consent to AI use of `provider`'s data.
///
/// `PUT /api/providers/{provider}/ai-consent` — records the current version of
/// the provider's notice, exactly as accepting it on a connect does.
///
/// # Errors
/// Unauthenticated; an admin's support session; no active tenant; a provider
/// whose notice is not a consent to AI use (`invalid_input`); or the store's
/// error.
pub(crate) async fn handle_grant_ai_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    give_consent(&resources, &provider, &headers, NoticeKind::AiConsent).await
}

/// Withdraw the account's consent to AI use of `provider`'s data.
///
/// `DELETE /api/providers/{provider}/ai-consent` — one call, and no model reads
/// that provider's data from the next read on. Withdrawing a consent already
/// withdrawn answers the same.
///
/// # Errors
/// Unauthenticated; a provider whose notice is not a consent to AI use
/// (`invalid_input`); or the store's error.
pub(crate) async fn handle_withdraw_ai_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    take_back_consent(&resources, &provider, &headers, NoticeKind::AiConsent).await
}

/// Give the account's consent to Dravr sending `provider` its data.
///
/// `PUT /api/providers/{provider}/transfer-consent` — records the current
/// version of the transfer notice, exactly as ticking it on a connect does.
///
/// # Errors
/// Unauthenticated; an admin's support session; no active tenant; a provider
/// that asks no transfer consent (`invalid_input`); or the store's error.
pub(crate) async fn handle_grant_transfer_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    give_consent(&resources, &provider, &headers, NoticeKind::TransferConsent).await
}

/// Withdraw the account's consent to Dravr sending `provider` its data.
///
/// `DELETE /api/providers/{provider}/transfer-consent` — one call, and Dravr
/// sends that provider nothing from then on.
///
/// # Errors
/// Unauthenticated; a provider that asks no transfer consent
/// (`invalid_input`); or the store's error.
pub(crate) async fn handle_withdraw_transfer_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    take_back_consent(&resources, &provider, &headers, NoticeKind::TransferConsent).await
}
