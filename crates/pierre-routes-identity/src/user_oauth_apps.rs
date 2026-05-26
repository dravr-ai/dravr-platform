// ABOUTME: User OAuth app management routes for per-user OAuth credentials
// ABOUTME: Enables users to configure their own OAuth app credentials to avoid rate limits
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User OAuth App Routes
//!
//! This module provides REST endpoints for users to manage their own OAuth
//! application credentials. Users can configure per-provider OAuth apps to:
//! - Avoid rate limits on shared tenant/server apps
//! - Use their own Strava/Fitbit/Garmin/WHOOP/Terra API applications
//!
//! ## Endpoints
//!
//! - `POST /api/users/oauth-apps` - Register a new OAuth app for current user
//! - `GET /api/users/oauth-apps` - List user's OAuth apps
//! - `GET /api/users/oauth-apps/:provider` - Get specific OAuth app
//! - `DELETE /api/users/oauth-apps/:provider` - Remove OAuth app

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{IdentityCtx, MiddlewareCtx};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

/// Routes for user OAuth app management
pub struct UserOAuthAppRoutes;

/// Request to register a new OAuth app for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterUserOAuthAppRequest {
    /// OAuth provider name (strava, fitbit, garmin, whoop, terra)
    pub provider: String,
    /// OAuth client ID from the provider
    pub client_id: String,
    /// OAuth client secret from the provider
    pub client_secret: String,
    /// OAuth redirect URI configured with the provider
    pub redirect_uri: String,
}

/// Response after registering an OAuth app
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterUserOAuthAppResponse {
    /// Whether the registration was successful
    pub success: bool,
    /// Provider name
    pub provider: String,
    /// Message describing the result
    pub message: String,
}

/// Summary of a user's OAuth app (without secret)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOAuthAppSummary {
    /// OAuth provider name
    pub provider: String,
    /// OAuth client ID (public)
    pub client_id: String,
    /// OAuth redirect URI
    pub redirect_uri: String,
    /// When this app was configured
    pub created_at: String,
}

/// Response listing user's OAuth apps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUserOAuthAppsResponse {
    /// List of configured OAuth apps
    pub apps: Vec<UserOAuthAppSummary>,
}

impl UserOAuthAppRoutes {
    /// Create all user OAuth app routes.
    ///
    /// Generic over the runtime context so the crate stays decoupled from
    /// `pierre-server`'s `ServerContext`.
    pub fn routes<C>() -> Router<Arc<C>>
    where
        C: IdentityCtx + MiddlewareCtx,
    {
        Router::new()
            .route("/api/users/oauth-apps", post(handle_register_app::<C>))
            .route("/api/users/oauth-apps", get(handle_list_apps::<C>))
            .route("/api/users/oauth-apps/{provider}", get(handle_get_app::<C>))
            .route(
                "/api/users/oauth-apps/{provider}",
                delete(handle_delete_app::<C>),
            )
    }
}

/// Validate provider name against the supported list.
fn validate_provider(provider: &str) -> Result<(), AppError> {
    const VALID_PROVIDERS: &[&str] = &["strava", "fitbit", "garmin", "whoop", "terra", "sciotte"];
    if VALID_PROVIDERS.contains(&provider.to_lowercase().as_str()) {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!(
            "Invalid provider '{}'. Valid providers: {}",
            provider,
            VALID_PROVIDERS.join(", ")
        )))
    }
}

/// Handle registering a new OAuth app
async fn handle_register_app<C: IdentityCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
    Json(request): Json<RegisterUserOAuthAppRequest>,
) -> Result<Response, AppError> {
    let user_id = auth.user_id;
    let provider = request.provider.to_lowercase();

    validate_provider(&provider)?;

    // Validate client_id and client_secret are not empty
    if request.client_id.trim().is_empty() {
        return Err(AppError::invalid_input("client_id cannot be empty"));
    }
    if request.client_secret.trim().is_empty() {
        return Err(AppError::invalid_input("client_secret cannot be empty"));
    }
    if request.redirect_uri.trim().is_empty() {
        return Err(AppError::invalid_input("redirect_uri cannot be empty"));
    }

    IdentityCtx::repos(resources.as_ref())
        .oauth_tokens
        .store_user_oauth_app(
            user_id,
            &provider,
            &request.client_id,
            &request.client_secret,
            &request.redirect_uri,
        )
        .await?;

    info!(
        user_id = %user_id,
        provider = %provider,
        "User registered OAuth app"
    );

    let response = RegisterUserOAuthAppResponse {
        success: true,
        provider: provider.clone(),
        message: format!(
            "OAuth app for {provider} registered successfully. Your API calls will now use your own credentials."
        ),
    };

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle listing user's OAuth apps
async fn handle_list_apps<C: IdentityCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let user_id = auth.user_id;

    let apps = IdentityCtx::repos(resources.as_ref())
        .oauth_tokens
        .list_user_oauth_apps(user_id)
        .await?;

    let summaries: Vec<UserOAuthAppSummary> = apps
        .into_iter()
        .map(|app| UserOAuthAppSummary {
            provider: app.provider,
            client_id: app.client_id,
            redirect_uri: app.redirect_uri,
            created_at: app.created_at.to_rfc3339(),
        })
        .collect();

    let response = ListUserOAuthAppsResponse { apps: summaries };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle getting a specific OAuth app
async fn handle_get_app<C: IdentityCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(provider): Path<String>,
) -> Result<Response, AppError> {
    let user_id = auth.user_id;
    let provider = provider.to_lowercase();

    validate_provider(&provider)?;

    let app = IdentityCtx::repos(resources.as_ref())
        .oauth_tokens
        .get_user_oauth_app(user_id, &provider)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!("No OAuth app configured for provider: {provider}"))
        })?;

    let summary = UserOAuthAppSummary {
        provider: app.provider,
        client_id: app.client_id,
        redirect_uri: app.redirect_uri,
        created_at: app.created_at.to_rfc3339(),
    };

    Ok((StatusCode::OK, Json(summary)).into_response())
}

/// Handle deleting an OAuth app
async fn handle_delete_app<C: IdentityCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(provider): Path<String>,
) -> Result<Response, AppError> {
    let user_id = auth.user_id;
    let provider = provider.to_lowercase();

    validate_provider(&provider)?;

    IdentityCtx::repos(resources.as_ref())
        .oauth_tokens
        .remove_user_oauth_app(user_id, &provider)
        .await?;

    info!(
        user_id = %user_id,
        provider = %provider,
        "User removed OAuth app"
    );

    Ok(StatusCode::NO_CONTENT.into_response())
}
