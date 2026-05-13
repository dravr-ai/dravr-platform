// ABOUTME: Fitness configuration route handlers for training settings
// ABOUTME: Provides REST endpoints for managing fitness configurations and training parameters
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fitness configuration routes
//!
//! This module handles fitness-specific configuration including training zones,
//! thresholds, and workout parameters. All handlers require valid JWT authentication.

use crate::{
    config::routes::fitness::{
        FitnessConfigurationRoutes as FitnessService, SaveFitnessConfigRequest,
    },
    errors::AppError,
    mcp::resources::ServerContext,
    middleware::AuthenticatedUser,
};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

/// Query parameters for fitness configuration endpoints
#[derive(Deserialize, Default)]
struct ConfigurationQuery {
    #[serde(default)]
    configuration_name: Option<String>,
}

impl ConfigurationQuery {
    fn get_name_or_default(&self) -> &str {
        self.configuration_name.as_deref().unwrap_or("default")
    }
}

/// Fitness configuration routes
pub struct FitnessConfigurationRoutes;

impl FitnessConfigurationRoutes {
    /// Create all fitness configuration routes
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/fitness/config", get(Self::handle_get_config))
            .route("/fitness/config", put(Self::handle_save_config))
            .route("/fitness/config", delete(Self::handle_delete_config))
            .with_state(resources)
    }

    /// Handle get fitness configuration
    async fn handle_get_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(params): Query<ConfigurationQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = FitnessService::new(resources);
        let response = service
            .get_configuration(&auth, params.get_name_or_default())
            .await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle save fitness configuration
    async fn handle_save_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Json(request): Json<SaveFitnessConfigRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = FitnessService::new(resources);
        let response = service.save_user_configuration(&auth, request).await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle delete fitness configuration
    async fn handle_delete_config(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(params): Query<ConfigurationQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let service = FitnessService::new(resources);
        service
            .delete_user_configuration(&auth, params.get_name_or_default())
            .await?;

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }
}
