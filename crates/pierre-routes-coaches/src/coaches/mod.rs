// ABOUTME: Module root for Coaches REST API route handlers
// ABOUTME: Defines coach user + admin router builders generic over CoachesCtx + MiddlewareCtx
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaches routes
//!
//! Handles coach endpoints for custom AI personas. All endpoints require
//! JWT authentication to identify the user and tenant.

mod admin;
mod handle;
/// Athlete-profile view + the pillar-context prompt block, shared by the
/// coach-proposal REST route and the messaging auto-send.
pub mod proposal_profile;
/// Request and response types for coach API endpoints.
pub mod types;
mod user;
mod versions;

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use chrono::Utc;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_database::backends::StoreListingsRepository;
use pierre_database::database::repositories::CoachesRepository;
use pierre_runtime_context::{CoachesCtx, MiddlewareCtx};
use pierre_tool_runtime::runtime::ToolRuntime;

pub use types::{
    CoachProposalResponse, CoachResponse, CoachesMetadata, CreateCoachBody, HideCoachResponse,
    ListCoachesQuery, ListCoachesResponse, ProposedCoach, RecordUsageResponse, SearchCoachesQuery,
    SportProfileSummary, SportShare, ToggleFavoriteResponse, UpdateCoachBody,
};
/// Shared coach-proposal builder — used by the REST route and the messaging
/// auto-send so both surfaces propose identically.
pub use user::build_coach_proposal;

/// Build the user-facing coaches router under `/api/coaches/{...}`.
pub fn build_coaches_router<C>() -> Router<Arc<C>>
where
    C: CoachesCtx + MiddlewareCtx + ToolRuntime,
{
    Router::new()
        .route("/api/coaches", get(user::handle_list::<C>))
        .route("/api/coaches", post(user::handle_create::<C>))
        .route("/api/coaches/search", get(user::handle_search::<C>))
        .route("/api/coaches/proposal", get(user::handle_proposal::<C>))
        .route("/api/coaches/hidden", get(user::handle_list_hidden::<C>))
        .route("/api/coaches/import", post(user::handle_import::<C>))
        .route(
            "/api/coaches/import/preview",
            post(user::handle_import_preview::<C>),
        )
        .route(
            "/api/coaches/import/url",
            post(user::handle_import_from_url::<C>),
        )
        .route("/api/coaches/generate", post(user::handle_generate::<C>))
        .route(
            "/api/coaches/by-handle/{handle}",
            get(handle::handle_get_by_handle::<C>),
        )
        .route("/api/coaches/{id}", get(user::handle_get::<C>))
        .route("/api/coaches/{id}", put(user::handle_update::<C>))
        .route("/api/coaches/{id}", delete(user::handle_delete::<C>))
        .route("/api/coaches/{id}/export", get(user::handle_export::<C>))
        .route(
            "/api/coaches/{id}/favorite",
            post(user::handle_toggle_favorite::<C>),
        )
        .route(
            "/api/coaches/{id}/usage",
            post(user::handle_record_usage::<C>),
        )
        .route("/api/coaches/{id}/hide", post(user::handle_hide_coach::<C>))
        .route(
            "/api/coaches/{id}/hide",
            delete(user::handle_show_coach::<C>),
        )
        .route("/api/coaches/{id}/fork", post(user::handle_fork::<C>))
        // Version history routes
        .route(
            "/api/coaches/{id}/versions",
            get(versions::handle_list_versions::<C>),
        )
        .route(
            "/api/coaches/{id}/versions/{version}/revert",
            post(versions::handle_revert_version::<C>),
        )
        .route(
            "/api/coaches/{id}/versions/{v1}/diff/{v2}",
            get(versions::handle_diff_versions::<C>),
        )
}

/// Build the admin coach router. Mount under `/api/admin`.
pub fn build_coaches_admin_router<C>() -> Router<Arc<C>>
where
    C: CoachesCtx + MiddlewareCtx,
{
    Router::new()
        .route("/coaches", get(admin::handle_admin_list::<C>))
        .route("/coaches", post(admin::handle_admin_create::<C>))
        .route("/coaches/{id}", get(admin::handle_admin_get::<C>))
        .route("/coaches/{id}", put(admin::handle_admin_update::<C>))
        .route("/coaches/{id}", delete(admin::handle_admin_delete::<C>))
        .route(
            "/coaches/{id}/assign",
            post(admin::handle_admin_assign::<C>),
        )
        .route(
            "/coaches/{id}/assign",
            delete(admin::handle_admin_unassign::<C>),
        )
        .route(
            "/coaches/{id}/assignments",
            get(admin::handle_admin_list_assignments::<C>),
        )
        // Store management routes
        .route("/store/stats", get(admin::handle_admin_store_stats::<C>))
        .route(
            "/store/review-queue",
            get(admin::handle_admin_review_queue::<C>),
        )
        .route("/store/published", get(admin::handle_admin_published::<C>))
        .route("/store/rejected", get(admin::handle_admin_rejected::<C>))
        .route(
            "/store/coaches/{id}/approve",
            post(admin::handle_admin_approve::<C>),
        )
        .route(
            "/store/coaches/{id}/reject",
            post(admin::handle_admin_reject::<C>),
        )
        .route(
            "/store/coaches/{id}/unpublish",
            post(admin::handle_admin_unpublish::<C>),
        )
}

// ============================================
// Shared Helper Functions
// ============================================

/// Get tenant ID for an authenticated user.
///
/// Extracts `active_tenant_id` from JWT claims (user's selected tenant).
/// Returns an error if no active tenant is set in the session.
pub(crate) fn get_user_tenant(auth: &AuthResult) -> Result<TenantId, AppError> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Get coaches repository from the runtime context.
pub(crate) fn get_coaches_manager<C: CoachesCtx>(ctx: &Arc<C>) -> &dyn CoachesRepository {
    ctx.repos().coaches.as_ref()
}

/// Get store listings repository from the runtime context.
pub(crate) fn get_store_manager<C: CoachesCtx>(ctx: &Arc<C>) -> &dyn StoreListingsRepository {
    ctx.repos().store_listings.as_ref()
}

/// Build metadata for coach responses.
pub(crate) fn build_metadata() -> types::CoachesMetadata {
    types::CoachesMetadata {
        timestamp: Utc::now().to_rfc3339(),
        api_version: "1.0".to_owned(),
    }
}
