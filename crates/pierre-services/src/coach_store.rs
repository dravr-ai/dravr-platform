// ABOUTME: Coach Store operations — browse, search, install — shared by the REST routes and MCP tools
// ABOUTME: One projection, one grade re-rank, one install path, so every surface sees the same store

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Store service
//!
//! The Store's read and install operations, independent of transport.
//!
//! Both surfaces that reach the marketplace enter here: the REST handlers
//! behind `/api/store/*` (`pierre-routes-coaches`) and the chat-callable
//! `store` tools (`pierre-tool-runtime`). The tools exist because the store
//! used to be reachable only from the web UI — `CHAT_CALLABLE_CATEGORIES`
//! named no store category, so no chat surface, in-app or messaging, could
//! browse or install a coach at all.
//!
//! Keeping the projection, the grade re-rank and the install call in one place
//! is what makes those two surfaces the same store: a coach that ranks third
//! in the web browse ranks third in chat, and installing from either writes
//! the same row and emits the same `coach.installed` attribution.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::pagination::StoreSortOrder;
use pierre_database::database::{CoachCategory, CoachWithListing};
use pierre_database::views::CoachRepos;
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::coach_grading::{compute_coach_grades, rerank_by_grade, DEFAULT_VERDICT_LIMIT};

/// Default page size when a caller does not ask for one.
pub const DEFAULT_STORE_PAGE_SIZE: u32 = 20;
/// Hard cap on a single page of store results.
pub const MAX_STORE_PAGE_SIZE: u32 = 100;

/// A published coach as every Store surface renders it.
///
/// A projection of [`CoachWithListing`]: the coach's own descriptive fields
/// plus the listing's marketplace fields. Deliberately without the system
/// prompt — browse and search list many coaches, and the prompt is the single
/// largest field on the row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreCoach {
    /// Unique coach identifier
    pub id: Uuid,
    /// Coach title
    pub title: String,
    /// Coach description
    pub description: Option<String>,
    /// Category for organization
    pub category: CoachCategory,
    /// Tags for discovery
    pub tags: Vec<String>,
    /// Sample prompts showing usage
    pub sample_prompts: Vec<String>,
    /// Token count estimate
    pub token_count: u32,
    /// Number of installations
    pub install_count: u32,
    /// Optional icon URL
    pub icon_url: Option<String>,
    /// When published (ISO 8601 format)
    pub published_at: Option<String>,
    /// Author ID (optional - for author profile linking)
    pub author_id: Option<String>,
    /// Addressable catalogue handle (`@handle`); see [`Coach::handle`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

impl From<CoachWithListing> for StoreCoach {
    fn from(cwl: CoachWithListing) -> Self {
        Self {
            id: cwl.coach.id,
            title: cwl.coach.title,
            description: cwl.coach.description,
            category: cwl.coach.category,
            tags: cwl.coach.tags,
            sample_prompts: cwl.coach.sample_prompts,
            token_count: cwl.coach.token_count,
            install_count: cwl.listing.install_count,
            icon_url: cwl.listing.icon_url,
            published_at: cwl.listing.published_at.map(|dt| dt.to_rfc3339()),
            author_id: cwl.listing.author_id,
            handle: cwl.coach.handle,
        }
    }
}

/// What a caller asks of [`browse_store`].
#[derive(Debug, Default, Clone)]
pub struct BrowseStoreParams<'a> {
    /// Restrict to one category. `None` browses every category.
    pub category: Option<CoachCategory>,
    /// Ordering applied before the grade re-rank.
    pub sort_by: StoreSortOrder,
    /// Page size; clamped to `1..=MAX_STORE_PAGE_SIZE`.
    pub limit: u32,
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<&'a str>,
}

/// One page of published coaches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorePage {
    /// The coaches on this page, already re-ranked by grade.
    pub coaches: Vec<StoreCoach>,
    /// Cursor for the next page, absent when this is the last one.
    pub next_cursor: Option<String>,
    /// Whether more coaches follow this page.
    pub has_more: bool,
}

/// Browse published coaches, newest first by default, re-ranked by coach grade.
///
/// Grades are computed from `viewer_tenant`'s recent claim verdicts so
/// low-quality coaches fall below higher-graded peers even when they shipped
/// with more installs. A grading failure degrades to the underlying
/// `install_count` ordering rather than failing the browse.
///
/// # Errors
///
/// Returns the underlying repository error when the listing page cannot be read.
pub async fn browse_store(
    repos: &CoachRepos,
    viewer_tenant: TenantId,
    params: &BrowseStoreParams<'_>,
) -> AppResult<StorePage> {
    let limit = params.limit.clamp(1, MAX_STORE_PAGE_SIZE);
    let page = repos
        .store_listings
        .get_published_coaches_cursor(params.category, params.sort_by, limit, params.cursor)
        .await?;

    let mut coaches: Vec<StoreCoach> = page.items.into_iter().map(StoreCoach::from).collect();
    apply_grade_rank(repos, viewer_tenant, &mut coaches).await;

    Ok(StorePage {
        coaches,
        next_cursor: page.next_cursor.map(|c| c.to_string()),
        has_more: page.has_more,
    })
}

/// Search published coaches by title, description, or tag.
///
/// The Store is global, so the search crosses tenants; `limit` is clamped to
/// `1..=MAX_STORE_PAGE_SIZE`.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] when `query` is
/// blank, and the underlying repository error when the search fails.
pub async fn search_store(
    repos: &CoachRepos,
    query: &str,
    limit: Option<u32>,
) -> AppResult<Vec<StoreCoach>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("Search query cannot be empty"));
    }
    let limit = limit
        .unwrap_or(DEFAULT_STORE_PAGE_SIZE)
        .clamp(1, MAX_STORE_PAGE_SIZE);
    let coaches = repos
        .store_listings
        .search_published_coaches(trimmed, Some(limit))
        .await?;
    Ok(coaches.into_iter().map(StoreCoach::from).collect())
}

/// Install a published coach, creating the caller's own copy.
///
/// The returned [`StoreCoach`] describes that copy: it has no listing of its
/// own, so `install_count` is 0 and the listing-only fields are absent.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] when `coach_id` is
/// not a UUID, and the underlying repository error when the install fails
/// (including when the coach is not published).
pub async fn install_store_coach(
    repos: &CoachRepos,
    coach_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<StoreCoach> {
    Uuid::parse_str(coach_id)
        .map_err(|_| AppError::invalid_input(format!("Invalid coach ID: {coach_id}")))?;

    let installed = repos
        .store_listings
        .install_from_store(coach_id, user_id, tenant_id)
        .await?;

    Ok(StoreCoach {
        id: installed.id,
        title: installed.title,
        description: installed.description,
        category: installed.category,
        tags: installed.tags,
        sample_prompts: installed.sample_prompts,
        token_count: installed.token_count,
        install_count: 0,
        icon_url: None,
        published_at: None,
        handle: installed.handle,
        author_id: None,
    })
}

/// Re-rank a page of store coaches by coach grade, degrading to the existing
/// order when grades cannot be computed.
async fn apply_grade_rank(repos: &CoachRepos, viewer_tenant: TenantId, coaches: &mut [StoreCoach]) {
    match compute_coach_grades(repos, viewer_tenant, DEFAULT_VERDICT_LIMIT).await {
        Ok(grading) => rerank_by_grade(coaches, |c| c.id.to_string(), &grading),
        Err(e) => {
            warn!(
                error = %e,
                "failed to compute coach grades for store rank; falling back to install_count"
            );
        }
    }
}
