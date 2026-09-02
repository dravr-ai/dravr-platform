// ABOUTME: Route handlers for Coach Store REST API (browse, search, install coaches)
// ABOUTME: Provides REST endpoints for Store discovery and installation operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach Store routes
//!
//! This module handles Store endpoints for discovering and installing coaches.
//! All endpoints require JWT authentication to identify the user and tenant.

use std::sync::Arc;

use crate::coaches::resolve_user_locale;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_core::pagination::StoreSortOrder;
use pierre_database::backends::StoreListingsRepository;
use pierre_database::database::{CoachCategory, CoachWithListing, PublishStatus};
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{CoachesCtx, MiddlewareCtx};
use pierre_services::coach_store::{
    browse_store, install_store_coach, search_store, translate_published_coach, BrowseStoreParams,
    StoreCoach, DEFAULT_STORE_PAGE_SIZE,
};
use serde::{Deserialize, Serialize};
use tracing::{field, info, Span};
use uuid::Uuid;

/// Query parameters for browsing published coaches
#[derive(Debug, Deserialize)]
pub struct BrowseCoachesQuery {
    /// Filter by category
    pub category: Option<String>,
    /// Sort by: "newest" (default), "popular", "title"
    pub sort_by: Option<String>,
    /// Maximum number of results (default 20, max 100)
    pub limit: Option<u32>,
    /// Encoded cursor for pagination (replaces offset)
    pub cursor: Option<String>,
}

/// Query parameters for searching coaches
#[derive(Debug, Deserialize)]
pub struct SearchCoachesQuery {
    /// Search query string
    pub q: String,
    /// Maximum number of results (default 20, max 100)
    pub limit: Option<u32>,
}

/// Full coach details for the Store (includes system prompt)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreCoachDetail {
    /// Basic store coach info
    #[serde(flatten)]
    pub coach: StoreCoach,
    /// System prompt (shown on detail page)
    pub system_prompt: String,
    /// When the coach was created (ISO 8601 format)
    pub created_at: String,
    /// Publish status
    pub publish_status: PublishStatus,
}

impl From<CoachWithListing> for StoreCoachDetail {
    fn from(cwl: CoachWithListing) -> Self {
        let system_prompt = cwl.coach.system_prompt.clone();
        let created_at = cwl.coach.created_at.to_rfc3339();
        let publish_status = cwl.listing.publish_status;
        Self {
            system_prompt,
            created_at,
            publish_status,
            coach: cwl.into(),
        }
    }
}

/// Category with coach count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryCount {
    /// Category identifier
    pub category: CoachCategory,
    /// Human-readable category name
    pub name: String,
    /// Number of published coaches in this category
    pub count: usize,
}

/// Response for browse endpoint with cursor-based pagination
#[derive(Debug, Serialize, Deserialize)]
pub struct BrowseCoachesResponse {
    /// List of coaches
    pub coaches: Vec<StoreCoach>,
    /// Cursor for fetching the next page (null if no more pages)
    pub next_cursor: Option<String>,
    /// Whether there are more items after this page
    pub has_more: bool,
    /// Response metadata
    pub metadata: StoreMetadata,
}

/// Response for search endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchCoachesResponse {
    /// Search results
    pub coaches: Vec<StoreCoach>,
    /// Search query that was used
    pub query: String,
    /// Response metadata
    pub metadata: StoreMetadata,
}

/// Response for categories endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct CategoriesResponse {
    /// Categories with counts
    pub categories: Vec<CategoryCount>,
    /// Response metadata
    pub metadata: StoreMetadata,
}

/// Response metadata
#[derive(Debug, Serialize, Deserialize)]
pub struct StoreMetadata {
    /// Response timestamp
    pub timestamp: String,
    /// API version
    pub api_version: String,
}

/// Build the coach-store router.
///
/// # Endpoints
///
/// - `GET /api/store/coaches` - Browse published coaches
/// - `GET /api/store/coaches/{id}` - Get coach details by ID
/// - `GET /api/store/categories` - List categories with counts
/// - `GET /api/store/search` - Search coaches
/// - `POST /api/store/coaches/{id}/install` - Install a coach
/// - `DELETE /api/store/coaches/{id}/install` - Uninstall a coach
/// - `GET /api/store/installations` - List user's installed coaches
pub fn build_store_router<C>() -> Router<Arc<C>>
where
    C: CoachesCtx + MiddlewareCtx,
{
    Router::new()
        .route("/api/store/health", get(store_health))
        .route("/api/store/coaches", get(handle_browse::<C>))
        .route("/api/store/coaches/{id}", get(handle_get_coach::<C>))
        .route(
            "/api/store/coaches/{id}/install",
            post(handle_install::<C>).delete(handle_uninstall::<C>),
        )
        .route("/api/store/categories", get(handle_categories::<C>))
        .route("/api/store/search", get(handle_search::<C>))
        .route(
            "/api/store/installations",
            get(handle_list_installations::<C>),
        )
}

/// Get tenant ID for an authenticated user
///
/// Extracts `active_tenant_id` from JWT claims (user's selected tenant).
/// Returns an error if no active tenant is set in the session.
fn get_user_tenant(auth: &AuthResult) -> Result<TenantId, AppError> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Get store listings repository from the runtime context.
fn get_store_manager<C: CoachesCtx>(ctx: &Arc<C>) -> &dyn StoreListingsRepository {
    ctx.repos().store_listings.as_ref()
}

/// Build response metadata
fn build_metadata() -> StoreMetadata {
    StoreMetadata {
        timestamp: Utc::now().to_rfc3339(),
        api_version: "1.0".to_owned(),
    }
}

/// Handle GET /api/store/coaches - Browse published coaches with cursor pagination
async fn handle_browse<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(query): Query<BrowseCoachesQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let viewer_tenant = get_user_tenant(&auth)?;

    let params = BrowseStoreParams {
        category: query.category.as_ref().map(|c| CoachCategory::parse(c)),
        sort_by: query
            .sort_by
            .as_deref()
            .map_or(StoreSortOrder::Newest, StoreSortOrder::parse),
        limit: query.limit.unwrap_or(DEFAULT_STORE_PAGE_SIZE),
        cursor: query.cursor.as_deref(),
    };
    // The athlete reads the catalogue in their own language: the listing
    // rows carry the English coach and the overlay adds the translation.
    let locale = resolve_user_locale(&ctx, auth.user_id, viewer_tenant).await;
    let page = browse_store(&ctx.repos().coach_repos(), viewer_tenant, &params, &locale).await?;

    info!(
        "User {} browsed store: {} coaches (category={:?}, sort={:?}, has_more={})",
        auth.user_id,
        page.coaches.len(),
        query.category,
        query.sort_by,
        page.has_more
    );

    let response = BrowseCoachesResponse {
        coaches: page.coaches,
        next_cursor: page.next_cursor,
        has_more: page.has_more,
        metadata: build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/store/coaches/{id} - Get coach details
async fn handle_get_coach<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(coach_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();

    let manager = get_store_manager(&ctx);

    // Parse coach ID to validate format
    Uuid::parse_str(&coach_id)
        .map_err(|_| AppError::invalid_input(format!("Invalid coach ID: {coach_id}")))?;

    // Get the published coach (cross-tenant - any published coach is visible)
    let mut cwl = manager
        .get_published_coach(&coach_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;
    let viewer_tenant = get_user_tenant(&auth)?;
    let locale = resolve_user_locale(&ctx, auth.user_id, viewer_tenant).await;
    translate_published_coach(&ctx.repos().coach_repos(), &mut cwl, &locale).await?;

    info!(
        "User {} viewed store coach: {} ({})",
        auth.user_id, cwl.coach.title, coach_id
    );

    let detail: StoreCoachDetail = cwl.into();
    Ok((StatusCode::OK, Json(detail)).into_response())
}

/// Handle GET /api/store/categories - List categories with counts
async fn handle_categories<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();

    let manager = get_store_manager(&ctx);

    // Use optimized single-query category count (replaces 7 queries with 1)
    let counts = manager.get_category_counts().await?;

    // Build response with all categories that have coaches
    let all_categories = [
        CoachCategory::Training,
        CoachCategory::Nutrition,
        CoachCategory::Recovery,
        CoachCategory::Recipes,
        CoachCategory::Mobility,
        CoachCategory::Analysis,
        CoachCategory::Custom,
    ];

    let categories: Vec<CategoryCount> = all_categories
        .iter()
        .filter_map(|cat| {
            counts.get(cat).and_then(|&count| {
                if count > 0 {
                    // Count from SQL COUNT is always non-negative and fits in usize
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let count_usize = count as usize;
                    Some(CategoryCount {
                        category: *cat,
                        name: cat.display_name().to_owned(),
                        count: count_usize,
                    })
                } else {
                    None
                }
            })
        })
        .collect();

    info!(
        "User {} fetched {} store categories",
        auth.user_id,
        categories.len()
    );

    let response = CategoriesResponse {
        categories,
        metadata: build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/store/search - Search published coaches
async fn handle_search<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(query): Query<SearchCoachesQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();

    // Search across all tenants (global Store), read in the athlete's language.
    let viewer_tenant = get_user_tenant(&auth)?;
    let locale = resolve_user_locale(&ctx, auth.user_id, viewer_tenant).await;
    let store_coaches =
        search_store(&ctx.repos().coach_repos(), &query.q, query.limit, &locale).await?;

    info!(
        "User {} searched store for '{}': {} results",
        auth.user_id,
        query.q,
        store_coaches.len()
    );

    let response = SearchCoachesResponse {
        coaches: store_coaches,
        query: query.q,
        metadata: build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/store/coaches/{id}/install - Install a coach from the Store
///
/// `coach.installed` is emitted by `install_store_coach`, the one install
/// path this route shares with the `install_coach_from_store` tool and
/// `/discover install`, so it fires once per install on every surface.
#[tracing::instrument(
    skip(ctx, auth),
    fields(route = "coach_install", coach_slug = %coach_id)
)]
async fn handle_install<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(coach_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_user_tenant(&auth)?;

    // Install the coach (creates user's copy)
    let store_coach = install_store_coach(
        &ctx.repos().coach_repos(),
        &coach_id,
        auth.user_id,
        tenant_id,
    )
    .await?;

    info!(
        "User {} installed coach '{}' ({}) from Store",
        auth.user_id, store_coach.title, coach_id
    );

    let response = InstallCoachResponse {
        message: format!("Successfully installed '{}'", store_coach.title),
        coach: store_coach,
        metadata: build_metadata(),
    };

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle DELETE /api/store/coaches/{id}/install - Uninstall a coach
#[tracing::instrument(
    skip(ctx, auth),
    fields(
        route = "coach_uninstall",
        coach_slug = %coach_id,
        user_id = field::Empty,
        tenant_id = field::Empty,
    )
)]
async fn handle_uninstall<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(coach_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_user_tenant(&auth)?;

    // Record IDs on the span so the NotifyLayer can attribute the
    // coach.uninstalled event without re-passing fields.
    let span = Span::current();
    span.record("user_id", field::display(&auth.user_id));
    span.record("tenant_id", field::display(&tenant_id));

    let manager = get_store_manager(&ctx);

    // Validate coach ID format
    Uuid::parse_str(&coach_id)
        .map_err(|_| AppError::invalid_input(format!("Invalid coach ID: {coach_id}")))?;

    // Uninstall the coach (deletes user's copy)
    let source_id = manager
        .uninstall_coach(&coach_id, auth.user_id, tenant_id)
        .await?;

    info!(
        "User {} uninstalled coach {} (source: {})",
        auth.user_id, coach_id, source_id
    );

    // notify: coach was successfully uninstalled (coach_slug is on the span).
    info!(
        target: "notify",
        event = "coach.uninstalled",
        "coach uninstalled"
    );

    let response = UninstallCoachResponse {
        message: "Coach uninstalled successfully".to_owned(),
        source_coach_id: source_id,
        metadata: build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/store/installations - List user's installed coaches
async fn handle_list_installations<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_user_tenant(&auth)?;

    let manager = get_store_manager(&ctx);

    let coaches = manager
        .get_installed_coaches(auth.user_id, tenant_id)
        .await?;

    // Installed coaches are personal copies without listing data
    let store_coaches: Vec<StoreCoach> = coaches
        .into_iter()
        .map(|c| StoreCoach {
            id: c.id,
            title: c.title,
            description: c.description,
            category: c.category,
            tags: c.tags,
            sample_prompts: c.sample_prompts,
            token_count: c.token_count,
            install_count: 0,
            icon_url: None,
            published_at: None,
            author_id: None,
            handle: c.handle,
        })
        .collect();

    info!(
        "User {} listed {} installed coaches",
        auth.user_id,
        store_coaches.len()
    );

    let response = InstallationsResponse {
        coaches: store_coaches,
        metadata: build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Response for install endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallCoachResponse {
    /// Success message
    pub message: String,
    /// The installed coach (user's copy)
    pub coach: StoreCoach,
    /// Response metadata
    pub metadata: StoreMetadata,
}

/// Response for uninstall endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct UninstallCoachResponse {
    /// Success message
    pub message: String,
    /// The source coach ID that was uninstalled
    pub source_coach_id: String,
    /// Response metadata
    pub metadata: StoreMetadata,
}

/// Response for installations list endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallationsResponse {
    /// User's installed coaches
    pub coaches: Vec<StoreCoach>,
    /// Response metadata
    pub metadata: StoreMetadata,
}

/// Health check endpoint for store routes
async fn store_health() -> &'static str {
    "Store routes healthy"
}
