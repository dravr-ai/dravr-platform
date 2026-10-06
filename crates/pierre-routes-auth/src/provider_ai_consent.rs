// ABOUTME: Give or withdraw the account's consent to AI use of one provider's data, from settings or the connection card
// ABOUTME: PUT and DELETE /api/providers/{provider}/ai-consent — withdrawal is one call, as simple as the consent (carnet#726)
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
//! back.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::errors::AppError;
use pierre_services::provider_notice::{grant_ai_consent, withdraw_ai_consent};
use serde_json::json;

use crate::AuthRoutesContext;

/// The provider as a refusal names it: its registered display name.
fn brand(resources: &AuthRoutesContext, provider: &str) -> String {
    resources
        .provider_registry
        .get_descriptor(provider)
        .map_or_else(|| provider.to_owned(), |d| d.display_name().to_owned())
}

/// Give the account's consent to AI use of `provider`'s data.
///
/// `PUT /api/providers/{provider}/ai-consent` — records the current version of
/// the provider's notice, exactly as accepting it on a connect does.
///
/// # Errors
/// Unauthenticated; no active tenant; a provider whose notice is not a
/// consent to AI use (`invalid_input`); or the store's error.
pub async fn handle_grant_ai_consent(
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
pub async fn handle_withdraw_ai_consent(
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
