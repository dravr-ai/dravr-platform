// ABOUTME: Module root for Coaches REST API route handlers
// ABOUTME: Defines CoachesRoutes router construction and shared helper functions used by sub-modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaches routes
//!
//! This module handles coach endpoints for custom AI personas.
//! All endpoints require JWT authentication to identify the user and tenant.

mod admin;
/// Request and response types for coach API endpoints
pub mod types;
mod user;
mod versions;

use crate::{errors::AppError, mcp::resources::ServerContext, models::TenantId};
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use chrono::Utc;
use pierre_auth::auth::AuthResult;
use pierre_database::backends::StoreListingsRepository;
use pierre_database::database::repositories::CoachesRepository;
use std::sync::Arc;

pub use types::{
    CoachResponse, CoachesMetadata, CreateCoachBody, HideCoachResponse, ListCoachesQuery,
    ListCoachesResponse, RecordUsageResponse, SearchCoachesQuery, ToggleFavoriteResponse,
    UpdateCoachBody,
};

/// Coaches routes handler
pub struct CoachesRoutes;

impl CoachesRoutes {
    /// Create all coaches routes
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/coaches", get(user::handle_list))
            .route("/api/coaches", post(user::handle_create))
            .route("/api/coaches/search", get(user::handle_search))
            .route("/api/coaches/hidden", get(user::handle_list_hidden))
            .route("/api/coaches/import", post(user::handle_import))
            .route(
                "/api/coaches/import/preview",
                post(user::handle_import_preview),
            )
            .route(
                "/api/coaches/import/url",
                post(user::handle_import_from_url),
            )
            .route("/api/coaches/generate", post(user::handle_generate))
            .route("/api/coaches/{id}", get(user::handle_get))
            .route("/api/coaches/{id}", put(user::handle_update))
            .route("/api/coaches/{id}", delete(user::handle_delete))
            .route("/api/coaches/{id}/export", get(user::handle_export))
            .route(
                "/api/coaches/{id}/favorite",
                post(user::handle_toggle_favorite),
            )
            .route("/api/coaches/{id}/usage", post(user::handle_record_usage))
            .route("/api/coaches/{id}/hide", post(user::handle_hide_coach))
            .route("/api/coaches/{id}/hide", delete(user::handle_show_coach))
            .route("/api/coaches/{id}/fork", post(user::handle_fork))
            // Version history routes
            .route(
                "/api/coaches/{id}/versions",
                get(versions::handle_list_versions),
            )
            .route(
                "/api/coaches/{id}/versions/{version}",
                get(versions::handle_get_version),
            )
            .route(
                "/api/coaches/{id}/versions/{version}/revert",
                post(versions::handle_revert_version),
            )
            .route(
                "/api/coaches/{id}/versions/{v1}/diff/{v2}",
                get(versions::handle_diff_versions),
            )
            .with_state(resources)
    }

    /// Create admin routes for system coaches management
    pub fn admin_routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/coaches", get(admin::handle_admin_list))
            .route("/coaches", post(admin::handle_admin_create))
            .route("/coaches/{id}", get(admin::handle_admin_get))
            .route("/coaches/{id}", put(admin::handle_admin_update))
            .route("/coaches/{id}", delete(admin::handle_admin_delete))
            .route("/coaches/{id}/assign", post(admin::handle_admin_assign))
            .route("/coaches/{id}/assign", delete(admin::handle_admin_unassign))
            .route(
                "/coaches/{id}/assignments",
                get(admin::handle_admin_list_assignments),
            )
            // Store management routes
            .route("/store/stats", get(admin::handle_admin_store_stats))
            .route("/store/review-queue", get(admin::handle_admin_review_queue))
            .route("/store/published", get(admin::handle_admin_published))
            .route("/store/rejected", get(admin::handle_admin_rejected))
            .route(
                "/store/coaches/{id}/approve",
                post(admin::handle_admin_approve),
            )
            .route(
                "/store/coaches/{id}/reject",
                post(admin::handle_admin_reject),
            )
            .route(
                "/store/coaches/{id}/unpublish",
                post(admin::handle_admin_unpublish),
            )
            .with_state(resources)
    }
}

// ============================================
// Shared Helper Functions
// ============================================

/// Get tenant ID for an authenticated user
///
/// Extracts `active_tenant_id` from JWT claims (user's selected tenant).
/// Returns an error if no active tenant is set in the session.
pub(super) fn get_user_tenant(auth: &AuthResult) -> Result<TenantId, AppError> {
    auth.active_tenant_id
        .map(TenantId::from)
        .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Get coaches repository from server resources
pub(super) fn get_coaches_manager(resources: &Arc<ServerContext>) -> &dyn CoachesRepository {
    resources.coaches_manager()
}

/// Get store listings repository from server resources
pub(super) fn get_store_manager(resources: &Arc<ServerContext>) -> &dyn StoreListingsRepository {
    resources.store_listings_repository()
}

/// Build metadata for responses
pub(super) fn build_metadata() -> types::CoachesMetadata {
    types::CoachesMetadata {
        timestamp: Utc::now().to_rfc3339(),
        api_version: "1.0".to_owned(),
    }
}
