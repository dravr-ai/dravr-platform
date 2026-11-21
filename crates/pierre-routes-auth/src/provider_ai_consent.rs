// ABOUTME: Read, give or withdraw the account's consent to AI use of one provider's data or one consent class, from settings
// ABOUTME: GET, PUT and DELETE /api/providers/{provider}/ai-consent — withdrawal is one call, as simple as the consent (carnet#726)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's consent to AI use of a provider's data.
//!
//! A provider whose notice is a consent to AI use (WHOOP's owner
//! authorization) lets the athlete withdraw it at any time, as simply as it
//! was given. One `DELETE` withdraws it: from then on no model reads that
//! provider's data, while the athlete still sees what is stored and the
//! connection stays live. One `PUT` gives it again, in the notice's current
//! version. The card's `ai_consent` on `GET /api/providers` reads the answer
//! back for a provider; `GET` here reads it for any key, a consent class
//! included (`sensitive_t2`, the athlete's health records, J2), which has no
//! card.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::constants::oauth_providers::sensitive_display_name;
use pierre_core::errors::AppError;
use pierre_services::provider_notice::{ai_consent_state, grant_ai_consent, withdraw_ai_consent};
use serde_json::json;

use crate::AuthRoutesContext;

/// The provider as a refusal names it: a consent class's fixed display name,
/// else the provider's registered display name, else the key.
fn brand(resources: &AuthRoutesContext, provider: &str) -> String {
    if let Some(name) = sensitive_display_name(provider) {
        return name.to_owned();
    }
    resources
        .provider_registry
        .get_descriptor(provider)
        .map_or_else(|| provider.to_owned(), |d| d.display_name().to_owned())
}

/// Read the account's consent to AI use of `provider`'s data.
///
/// `GET /api/providers/{provider}/ai-consent` — `ai_consent` is `true` while
/// it is given in the notice's current version, `false` while it is not, and
/// `null` for a key that asks no such consent of this account.
///
/// # Errors
/// Unauthenticated; or no active tenant for this session.
pub(crate) async fn handle_get_ai_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;
    let tenant_id = auth
        .active_tenant_id
        .ok_or_else(|| AppError::invalid_input("No active tenant for this session"))?;
    let state = ai_consent_state(&resources.repos, tenant_id, auth.user_id, &provider).await;
    Ok((
        StatusCode::OK,
        Json(json!({ "provider": provider, "ai_consent": state })),
    )
        .into_response())
}

/// Give the account's consent to AI use of `provider`'s data.
///
/// `PUT /api/providers/{provider}/ai-consent` — records the current version of
/// the provider's notice, exactly as accepting it on a connect does.
///
/// # Errors
/// Unauthenticated; no active tenant; a provider whose notice is not a
/// consent to AI use (`invalid_input`); or the store's error.
pub(crate) async fn handle_grant_ai_consent(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;
    // A consent is the athlete's to give; an admin's support session may
    // withdraw one, never give it (carnet#1634).
    auth.refuse_impersonation(
        resources.repos.admin_audit.as_ref(),
        "give consent to AI use of a provider's data",
    )
    .await?;
    let tenant_id = auth
        .active_tenant_id
        .ok_or_else(|| AppError::invalid_input("No active tenant for this session"))?;
    let brand = brand(&resources, &provider);
    grant_ai_consent(&resources.repos, tenant_id, auth.user_id, &provider, &brand).await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "provider": provider, "ai_consent": true })),
    )
        .into_response())
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
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;
    let brand = brand(&resources, &provider);
    withdraw_ai_consent(&resources.repos, auth.user_id, &provider, &brand).await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "provider": provider, "ai_consent": false })),
    )
        .into_response())
}
