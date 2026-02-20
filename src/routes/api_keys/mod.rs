// ABOUTME: API key management route handlers for user self-service key operations
// ABOUTME: Provides REST endpoints for creating, listing, and managing API keys with authentication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! API key management routes
//!
//! This module handles API key creation, listing, deactivation, and usage tracking
//! for authenticated users. All handlers require valid JWT authentication.

/// Service layer for API key management operations
pub mod service;

use crate::{
    api_keys::CreateApiKeyRequestSimple, errors::AppError, mcp::resources::ServerResources,
    middleware::AuthenticatedUser,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use service::ApiKeyRoutes as ApiKeyService;
use std::sync::Arc;

/// Query parameters for API key usage statistics
#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    /// Start date for usage statistics (ISO 8601 format)
    pub start_date: DateTime<Utc>,
    /// End date for usage statistics (ISO 8601 format)
    pub end_date: DateTime<Utc>,
}

/// API key management routes
pub struct ApiKeyRoutes;

impl ApiKeyRoutes {
    /// Create all API key management routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route("/api/keys", post(Self::handle_create_api_key))
            .route("/api/keys", get(Self::handle_list_api_keys))
            .route("/api/keys/:key_id", delete(Self::handle_deactivate_api_key))
            .route("/api/keys/:key_id/usage", get(Self::handle_get_usage))
            .with_state(resources)
    }

    /// Handle API key creation
    async fn handle_create_api_key(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Json(request): Json<CreateApiKeyRequestSimple>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Create API key using service layer
        let service = ApiKeyService::new(resources);
        let response = service
            .create_api_key_simple(&auth, request)
            .await
            .map_err(|e| AppError::internal(format!("Failed to create API key: {e}")))?;

        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Handle listing user's API keys
    async fn handle_list_api_keys(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // List API keys using service layer
        let service = ApiKeyService::new(resources);
        let response = service
            .list_api_keys(&auth)
            .await
            .map_err(|e| AppError::internal(format!("Failed to list API keys: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle API key deactivation
    async fn handle_deactivate_api_key(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(key_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Deactivate API key using service layer
        let service = ApiKeyService::new(resources);
        let response = service
            .deactivate_api_key(&auth, &key_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to deactivate API key: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle getting API key usage statistics
    async fn handle_get_usage(
        State(resources): State<Arc<ServerResources>>,
        Path(key_id): Path<String>,
        Query(query): Query<UsageQuery>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Use the service layer which enforces ownership verification
        let service = ApiKeyService::new(resources);
        let response = service
            .get_api_key_usage(&auth, &key_id, query.start_date, query.end_date)
            .await
            .map_err(|e| AppError::internal(format!("Failed to get API key usage: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
