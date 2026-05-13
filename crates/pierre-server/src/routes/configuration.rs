// ABOUTME: Configuration management route handlers for user fitness settings
// ABOUTME: Provides REST endpoints for managing user fitness configurations and training zones
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Configuration management routes
//!
//! This module handles fitness configuration management including training zones,
//! thresholds, and personalized settings. All handlers require valid JWT authentication.

use crate::{
    config::routes::configuration::{
        ConfigurationRoutes as ConfigService, UpdateConfigurationRequest,
    },
    errors::AppError,
    mcp::resources::ServerContext,
    middleware::AuthenticatedUser,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use std::sync::Arc;

/// Configuration management routes
pub struct ConfigurationRoutes;

impl ConfigurationRoutes {
    /// Create all configuration management routes
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/config", get(Self::handle_get_config))
            .route("/config", put(Self::handle_update_config))
            .route("/config/user", get(Self::handle_get_user_config))
            .route("/config/user", put(Self::handle_update_user_config))
            .with_state(resources)
    }

    /// Handle get configuration
    async fn handle_get_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = ConfigService::new(resources);
        let response = service
            .get_user_configuration(&auth)
            .await
            .map_err(|e| AppError::internal(format!("Failed to get user configuration: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle update configuration
    async fn handle_update_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Json(request): Json<UpdateConfigurationRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = ConfigService::new(resources);
        let response = service
            .update_user_configuration(&auth, request)
            .await
            .map_err(|e| AppError::internal(format!("Failed to update user configuration: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle get user-specific configuration
    async fn handle_get_user_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = ConfigService::new(resources);
        let response = service
            .get_user_configuration(&auth)
            .await
            .map_err(|e| AppError::internal(format!("Failed to get user configuration: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle update user-specific configuration
    async fn handle_update_user_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Json(request): Json<UpdateConfigurationRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = ConfigService::new(resources);
        let response = service
            .update_user_configuration(&auth, request)
            .await
            .map_err(|e| AppError::internal(format!("Failed to update user configuration: {e}")))?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
