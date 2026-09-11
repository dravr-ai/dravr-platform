// ABOUTME: Agent Store operations — browse, search, install — shared by the REST routes and MCP tools
// ABOUTME: One projection, one grade re-rank, one install path, so every surface sees the same store

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent Store service
//!
//! The Store's read and install operations, independent of transport.
//!
//! Both surfaces that reach the marketplace enter here: the REST handlers
//! behind `/api/store/*` (`pierre-routes-agents`) and the chat-callable
//! `store` tools (`pierre-tool-runtime`). The tools exist because the store
//! used to be reachable only from the web UI — `CHAT_CALLABLE_CATEGORIES`
//! named no store category, so no chat surface, in-app or messaging, could
//! browse or install an agent at all.
//!
//! Keeping the projection, the grade re-rank and the install call in one place
//! is what makes those surfaces the same store: an agent that ranks third in
//! the web browse ranks third in chat, and installing from any of them — the
//! REST route, the `install_agent_from_store` tool, `/discover install` —
//! writes the same row and emits `agent.installed` exactly once, from here.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::pagination::StoreSortOrder;
use std::slice;

use pierre_database::database::{Agent, AgentCategory, AgentWithListing, StoreListing};
use pierre_database::views::AgentRepos;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent_grading::{compute_agent_grades, rerank_by_grade, DEFAULT_VERDICT_LIMIT};

/// Default page size when a caller does not ask for one.
pub const DEFAULT_STORE_PAGE_SIZE: u32 = 20;
/// Hard cap on a single page of store results.
pub const MAX_STORE_PAGE_SIZE: u32 = 100;

/// A published agent as every Store surface renders it.
///
/// A projection of [`AgentWithListing`]: the agent's own descriptive fields
/// plus the listing's marketplace fields. Deliberately without the system
/// prompt — browse and search list many agents, and the prompt is the single
/// largest field on the row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreAgent {
    /// Unique agent identifier
    pub id: Uuid,
    /// Agent title
    pub title: String,
    /// Agent description
    pub description: Option<String>,
    /// Category for organization
    pub category: AgentCategory,
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
    /// Addressable catalogue handle (`@handle`); see [`Agent::handle`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

impl From<AgentWithListing> for StoreAgent {
    fn from(cwl: AgentWithListing) -> Self {
        Self {
            id: cwl.agent.id,
            title: cwl.agent.title,
            description: cwl.agent.description,
            category: cwl.agent.category,
            tags: cwl.agent.tags,
            sample_prompts: cwl.agent.sample_prompts,
            token_count: cwl.agent.token_count,
            install_count: cwl.listing.install_count,
            icon_url: cwl.listing.icon_url,
            published_at: cwl.listing.published_at.map(|dt| dt.to_rfc3339()),
            author_id: cwl.listing.author_id,
            handle: cwl.agent.handle,
        }
    }
}

/// What a caller asks of [`browse_store`].
#[derive(Debug, Default, Clone)]
pub struct BrowseStoreParams<'a> {
    /// Restrict to one category. `None` browses every category.
    pub category: Option<AgentCategory>,
    /// Ordering applied before the grade re-rank.
    pub sort_by: StoreSortOrder,
    /// Page size; clamped to `1..=MAX_STORE_PAGE_SIZE`.
    pub limit: u32,
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<&'a str>,
}

/// One page of published agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorePage {
    /// The agents on this page, already re-ranked by grade.
    pub agents: Vec<StoreAgent>,
    /// Cursor for the next page, absent when this is the last one.
    pub next_cursor: Option<String>,
    /// Whether more agents follow this page.
    pub has_more: bool,
}

/// Browse published agents, newest first by default, re-ranked by agent grade.
///
/// Grades are computed from `viewer_tenant`'s recent claim verdicts so
/// low-quality agents fall below higher-graded peers even when they shipped
/// with more installs. A grading failure degrades to the underlying
/// `install_count` ordering rather than failing the browse.
///
/// # Errors
///
/// Returns the underlying repository error when the listing page cannot be read.
pub async fn browse_store(
    repos: &AgentRepos,
    viewer_tenant: TenantId,
    params: &BrowseStoreParams<'_>,
    locale: &str,
) -> AppResult<StorePage> {
    let limit = params.limit.clamp(1, MAX_STORE_PAGE_SIZE);
    let page = repos
        .store_listings
        .get_published_agents_cursor(params.category, params.sort_by, limit, params.cursor)
        .await?;

    let items = translate_listings(repos, page.items, locale).await?;
    let mut agents: Vec<StoreAgent> = items.into_iter().map(StoreAgent::from).collect();
    apply_grade_rank(repos, viewer_tenant, &mut agents).await;

    Ok(StorePage {
        agents,
        next_cursor: page.next_cursor.map(|c| c.to_string()),
        has_more: page.has_more,
    })
}

/// One page of published agents addressed by offset.
///
/// Chat surfaces page with a plain offset carried in a button value
/// (`/discover more 8 training`), where a cursor would not fit Telegram's
/// 64-byte callback budget next to the command and a category.
#[derive(Debug, Clone)]
pub struct StoreOffsetPage {
    /// The agents on this page, already re-ranked by grade.
    pub agents: Vec<StoreAgent>,
    /// Offset of the page after this one, absent when this is the last.
    pub next_offset: Option<u32>,
}

/// Browse published agents newest first from `offset`, re-ranked by agent grade.
///
/// Reads one row past `limit` to learn whether a next page exists, so a
/// caller never offers a "More" that turns out empty. `limit` is clamped to
/// `1..MAX_STORE_PAGE_SIZE` — one under the cap, so the look-ahead row still
/// fits the repository's own page ceiling. Grading degrades exactly as in
/// [`browse_store`].
///
/// # Errors
///
/// Returns the underlying repository error when the listing page cannot be read.
pub async fn browse_store_page(
    repos: &AgentRepos,
    viewer_tenant: TenantId,
    category: Option<AgentCategory>,
    offset: u32,
    limit: u32,
    locale: &str,
) -> AppResult<StoreOffsetPage> {
    let limit = limit.clamp(1, MAX_STORE_PAGE_SIZE - 1);
    let page_len = limit as usize;
    let mut rows = repos
        .store_listings
        .get_published_agents(category, Some("newest"), Some(limit + 1), Some(offset))
        .await?;
    let has_more = rows.len() > page_len;
    rows.truncate(page_len);

    let rows = translate_listings(repos, rows, locale).await?;
    let mut agents: Vec<StoreAgent> = rows.into_iter().map(StoreAgent::from).collect();
    apply_grade_rank(repos, viewer_tenant, &mut agents).await;

    Ok(StoreOffsetPage {
        agents,
        next_offset: has_more.then(|| offset.saturating_add(limit)),
    })
}

/// Search published agents by title, description, or tag, in `locale`.
///
/// The Store is global, so the search crosses tenants; `limit` is clamped to
/// `1..=MAX_STORE_PAGE_SIZE`.
///
/// Tags are both what the chips show and what the search matches, and the
/// chips are localized — so the query runs against the canonical English row
/// *and* the athlete's own overlay. A French athlete searching the words the
/// Store showed her reaches the agent whose canonical slug is the English
/// one, and an English slug still finds the agents that carry it.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] when `query` is
/// blank, and the underlying repository error when the search fails.
pub async fn search_store(
    repos: &AgentRepos,
    query: &str,
    limit: Option<u32>,
    locale: &str,
) -> AppResult<Vec<StoreAgent>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("Search query cannot be empty"));
    }
    let limit = limit
        .unwrap_or(DEFAULT_STORE_PAGE_SIZE)
        .clamp(1, MAX_STORE_PAGE_SIZE);
    let agents = repos
        .store_listings
        .search_published_agents(trimmed, Some(limit), locale)
        .await?;
    let agents = translate_listings(repos, agents, locale).await?;
    Ok(agents.into_iter().map(StoreAgent::from).collect())
}

/// Install a published agent, creating the caller's own copy.
///
/// The returned [`StoreAgent`] describes that copy: it has no listing of its
/// own, so `install_count` is 0 and the listing-only fields are absent.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::invalid_input`] when `agent_id` is
/// not a UUID, and the underlying repository error when the install fails
/// (including when the agent is not published).
pub async fn install_store_agent(
    repos: &AgentRepos,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<StoreAgent> {
    Uuid::parse_str(agent_id)
        .map_err(|_| AppError::invalid_input(format!("Invalid coach ID: {agent_id}")))?;

    let installed = repos
        .store_listings
        .install_from_store(agent_id, user_id, tenant_id)
        .await?;

    // notify: the one emission for every install surface — the REST route,
    // the `install_agent_from_store` tool and `/discover install` all land
    // here, so an install counts exactly once whichever way it came in.
    // Fields are inline because a tool call or a slash command has no route
    // span to carry them.
    info!(
        target: "notify",
        event = "agent.installed",
        user_id = %user_id,
        tenant_id = %tenant_id,
        agent_slug = %agent_id,
        "coach installed from store"
    );

    Ok(StoreAgent {
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

/// Re-rank a page of store agents by agent grade, degrading to the existing
/// order when grades cannot be computed.
/// Overlay the `agent_translations` rows for `locale` onto listing agents.
///
/// The store reads the canonical English row; a French athlete browsing it
/// deserves the same French title and description the chat's `/agent list`
/// already shows. The agents are lifted out of their listings, translated
/// as one batch, and zipped back so the listing data rides along untouched.
async fn translate_listings(
    repos: &AgentRepos,
    items: Vec<AgentWithListing>,
    locale: &str,
) -> AppResult<Vec<AgentWithListing>> {
    let (mut agents, listings): (Vec<Agent>, Vec<StoreListing>) = items
        .into_iter()
        .map(|item| (item.agent, item.listing))
        .unzip();
    repos.agents.translate_agents(&mut agents, locale).await?;
    Ok(agents
        .into_iter()
        .zip(listings)
        .map(|(agent, listing)| AgentWithListing { agent, listing })
        .collect())
}

/// Overlay one published agent's translation for the detail read.
///
/// # Errors
///
/// Returns the repository error when the translation read fails.
pub async fn translate_published_agent(
    repos: &AgentRepos,
    agent: &mut AgentWithListing,
    locale: &str,
) -> AppResult<()> {
    repos
        .agents
        .translate_agents(slice::from_mut(&mut agent.agent), locale)
        .await
}

async fn apply_grade_rank(repos: &AgentRepos, viewer_tenant: TenantId, agents: &mut [StoreAgent]) {
    match compute_agent_grades(repos, viewer_tenant, DEFAULT_VERDICT_LIMIT).await {
        Ok(grading) => rerank_by_grade(agents, |c| c.id.to_string(), &grading),
        Err(e) => {
            warn!(
                error = %e,
                "failed to compute coach grades for store rank; falling back to install_count"
            );
        }
    }
}
