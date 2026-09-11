// ABOUTME: Module root for Coaches REST API route handlers
// ABOUTME: Defines agent user + admin router builders generic over AgentsCtx + MiddlewareCtx
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaches routes
//!
//! Handles agent endpoints for custom AI personas. All endpoints require
//! JWT authentication to identify the user and tenant.

mod admin;
mod handle;
/// Athlete-profile view + the pillar-context prompt block, shared by the
/// agent-proposal REST route and the messaging auto-send.
pub mod proposal_profile;
/// Request and response types for agent API endpoints.
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
use pierre_database::database::repositories::AgentsRepository;
use pierre_runtime_context::{AgentsCtx, MiddlewareCtx};
use pierre_tool_runtime::runtime::ToolRuntime;

pub use types::{
    AgentProposalResponse, AgentResponse, AgentsMetadata, CreateAgentBody, HideAgentResponse,
    ListAgentsQuery, ListAgentsResponse, ProposedAgent, RecordUsageResponse, SearchAgentsQuery,
    SportProfileSummary, SportShare, ToggleFavoriteResponse, UpdateAgentBody,
};
/// Shared agent-proposal builder — used by the REST route and the messaging
/// auto-send so both surfaces propose identically.
pub use user::build_agent_proposal;

/// Build the user-facing agents router under `/api/agents/{...}`.
pub fn build_agents_router<C>() -> Router<Arc<C>>
where
    C: AgentsCtx + MiddlewareCtx + ToolRuntime,
{
    Router::new()
        .route("/api/agents", get(user::handle_list::<C>))
        .route("/api/agents", post(user::handle_create::<C>))
        // Served to no client on either surface. The mobile agent library
        // filters and toggles hidden client-side over the already-fetched
        // list, and the same capability is reachable over MCP. Delete-or-wire
        // is the open decision on the issue.
        // LIMITATION(registre#387): /api/agents/search has no client
        // LIMITATION(registre#387): /api/agents/hidden has no client
        // LIMITATION(registre#387): /api/agents/import has no client
        .route("/api/agents/search", get(user::handle_search::<C>))
        .route("/api/agents/proposal", get(user::handle_proposal::<C>))
        .route("/api/agents/hidden", get(user::handle_list_hidden::<C>))
        .route("/api/agents/import", post(user::handle_import::<C>))
        .route(
            "/api/agents/import/preview",
            post(user::handle_import_preview::<C>),
        )
        .route(
            "/api/agents/import/url",
            post(user::handle_import_from_url::<C>),
        )
        .route(
            "/api/agents/by-handle/{handle}",
            get(handle::handle_get_by_handle::<C>),
        )
        .route("/api/agents/{id}", get(user::handle_get::<C>))
        .route("/api/agents/{id}", put(user::handle_update::<C>))
        .route("/api/agents/{id}", delete(user::handle_delete::<C>))
        .route("/api/agents/{id}/export", get(user::handle_export::<C>))
        .route(
            "/api/agents/{id}/favorite",
            post(user::handle_toggle_favorite::<C>),
        )
        .route(
            "/api/agents/{id}/usage",
            post(user::handle_record_usage::<C>),
        )
        .route("/api/agents/{id}/hide", post(user::handle_hide_agent::<C>))
        .route(
            "/api/agents/{id}/hide",
            delete(user::handle_show_agent::<C>),
        )
        .route("/api/agents/{id}/fork", post(user::handle_fork::<C>))
        // Version history routes
        .route(
            "/api/agents/{id}/versions",
            get(versions::handle_list_versions::<C>),
        )
        .route(
            "/api/agents/{id}/versions/{version}/revert",
            post(versions::handle_revert_version::<C>),
        )
        .route(
            "/api/agents/{id}/versions/{v1}/diff/{v2}",
            get(versions::handle_diff_versions::<C>),
        )
}

/// Build the admin agent router. Mount under `/api/admin`.
pub fn build_agents_admin_router<C>() -> Router<Arc<C>>
where
    C: AgentsCtx + MiddlewareCtx + ToolRuntime,
{
    Router::new()
        .route("/agents", get(admin::handle_admin_list::<C>))
        .route("/agents", post(admin::handle_admin_create::<C>))
        .route("/agents/{id}", get(admin::handle_admin_get::<C>))
        .route("/agents/{id}", put(admin::handle_admin_update::<C>))
        .route("/agents/{id}", delete(admin::handle_admin_delete::<C>))
        .route("/agents/{id}/assign", post(admin::handle_admin_assign::<C>))
        .route(
            "/agents/{id}/assign",
            delete(admin::handle_admin_unassign::<C>),
        )
        .route(
            "/agents/{id}/assignments",
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
            "/store/agents/{id}/approve",
            post(admin::handle_admin_approve::<C>),
        )
        .route(
            "/store/agents/{id}/reject",
            post(admin::handle_admin_reject::<C>),
        )
        .route(
            "/store/agents/{id}/unpublish",
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

/// Get agents repository from the runtime context.
pub(crate) fn get_agents_manager<C: AgentsCtx>(ctx: &Arc<C>) -> &dyn AgentsRepository {
    ctx.repos().agents.as_ref()
}

/// Get store listings repository from the runtime context.
pub(crate) fn get_store_manager<C: AgentsCtx>(ctx: &Arc<C>) -> &dyn StoreListingsRepository {
    ctx.repos().store_listings.as_ref()
}

/// Build metadata for agent responses.
pub(crate) fn build_metadata() -> types::AgentsMetadata {
    types::AgentsMetadata {
        timestamp: Utc::now().to_rfc3339(),
        api_version: "1.0".to_owned(),
    }
}
