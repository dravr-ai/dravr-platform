// ABOUTME: Coach Store tools — browse, search and install a published coach from any chat surface.
// ABOUTME: Thin MCP shells over pierre_services::coach_store, the same code the /api/store routes run.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Store tools
//!
//! - [`BrowseCoachStoreTool`] — page through published coaches, grade-ranked
//! - [`SearchCoachStoreTool`] — search published coaches by text
//! - [`InstallCoachFromStoreTool`] — install one into the caller's library
//!
//! Registered under the `store` category, which is chat-callable: the store
//! was previously reachable only from the web UI, because
//! [`ToolRegistry::chat_callable_schemas`](crate::registry::ToolRegistry::chat_callable_schemas)
//! named no store category at all. An athlete asking their coach "what
//! nutrition coaches are there?" got a truthful refusal on web, mobile and
//! messaging alike.
//!
//! Every operation delegates to [`pierre_services::coach_store`], which the
//! `/api/store/*` REST handlers also call, so the ranking an athlete sees in
//! chat is the ranking the web store shows.
//!
//! Install is a real, reversible write: it creates the caller's own copy of a
//! published coach. Uninstall is deliberately *not* exposed here — removing a
//! coach an athlete may have conversation history with is a destructive act
//! that belongs to a deliberate UI gesture, not to an LLM's reading of a
//! sentence.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::info;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::CoachCategory;
use pierre_core::models::TenantId;
use pierre_core::pagination::StoreSortOrder;
use pierre_mcp_schema::PropertySchema;
use pierre_services::coach_store::{
    browse_store, install_store_coach, search_store, BrowseStoreParams, StoreCoach,
    DEFAULT_STORE_PAGE_SIZE, MAX_STORE_PAGE_SIZE,
};
use pierre_services::locale::resolve_user_locale;
use pierre_tools_core::ToolResult;

use super::coaches_tool_shape::{extract_format, read_only_annotations, write_annotations};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;

/// The language the athlete reads the catalogue in — their stored locale,
/// or the platform default when the row does not say.
async fn athlete_locale(context: &ToolExecutionContext) -> String {
    resolve_user_locale(
        context.resources.data().repos().users.as_ref(),
        context.user_id,
    )
    .await
}

/// Factory for the Coach Store tools, registered under the `store` category.
#[must_use]
pub fn create_store_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(BrowseCoachStoreTool),
        Box::new(SearchCoachStoreTool),
        Box::new(InstallCoachFromStoreTool),
    ]
}

/// Project one store coach into the compact shape a coaching turn reasons
/// over. The system prompt is never included: browse and search return many
/// coaches, and the prompt is by far the largest field on the row.
fn project(coach: &StoreCoach) -> StoreCoachEntry {
    StoreCoachEntry {
        id: coach.id.to_string(),
        title: coach.title.clone(),
        description: coach.description.clone(),
        category: coach.category.as_str().to_owned(),
        tags: coach.tags.clone(),
        sample_prompts: coach.sample_prompts.clone(),
        install_count: coach.install_count,
        published_at: coach.published_at.clone(),
    }
}

/// One store coach as the browse, search and install tools report it.
///
/// The system prompt is deliberately absent. Browse and search return many
/// coaches and the prompt is by far the largest field on the row; install
/// echoes the same compact shape so a client renders one card either way.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StoreCoachEntry {
    /// Identifier `install_coach_from_store` takes.
    pub id: String,
    /// Display name, as its author wrote it.
    pub title: String,
    /// What the coach is for; absent when its author gave none.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels its author set.
    pub tags: Vec<String>,
    /// Openers the author suggests, for a client to offer as chips.
    pub sample_prompts: Vec<String>,
    /// How many athletes have installed it.
    pub install_count: u32,
    /// RFC 3339 timestamp of publication; absent while unpublished.
    pub published_at: Option<String>,
}

/// What `browse_coach_store` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct BrowseCoachStoreResult {
    /// The coaches on this page.
    pub coaches: Vec<StoreCoachEntry>,
    /// How many came back.
    pub count: usize,
    /// Whether another page follows.
    pub has_more: bool,
    /// The cursor that fetches it; absent on the last page.
    pub next_cursor: Option<String>,
}

/// What `search_coach_store` answers with.
///
/// No cursor: search is a single ranked page, so there is nothing to page to.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchCoachStoreResult {
    /// The query, echoed back.
    pub query: String,
    /// How many matched.
    pub count: usize,
    /// The matches.
    pub coaches: Vec<StoreCoachEntry>,
}

/// What `install_coach_from_store` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct InstallCoachFromStoreResult {
    /// Always true: the tool errors rather than reporting a failed install.
    pub installed: bool,
    /// The installed copy, in the same shape browse and search send.
    pub coach: StoreCoachEntry,
    /// What to tell the athlete, already written for them.
    pub message: String,
}

/// Read `limit` from tool arguments, clamped to the store's page bounds.
fn limit_arg(args: &Value) -> u32 {
    args.get("limit")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_STORE_PAGE_SIZE)
        .clamp(1, MAX_STORE_PAGE_SIZE)
}

/// Browse published coaches in the Coach Store.
pub struct BrowseCoachStoreTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for BrowseCoachStoreTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional category filter: training, nutrition, recovery, recipes, mobility, \
                     analysis, or custom."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sort_by".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Ordering: 'newest' (default), 'popular' (most installed), or 'title'."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Coaches per page (1-100, default 20).".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "cursor".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Opaque cursor from a previous call's `next_cursor`, to fetch the next page."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);

        answers_with::<Formatted<BrowseCoachStoreResult>>(tool_definition(
            "browse_coach_store",
            "Browse the Coach Store — the catalogue of PUBLISHED coaches anyone can install. Use \
             this when the athlete asks what coaches exist, or for a coach of a given kind they do \
             not already have. Distinct from `list_coaches`, which lists only the coaches ALREADY \
             in their library. Returns a page plus a `next_cursor` for the following one.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let viewer_tenant = TenantId::from_uuid(context.require_tenant()?);
            let locale = athlete_locale(&context).await;
            let repos = context.resources.data().repos().coach_repos();

            let cursor = args.get("cursor").and_then(Value::as_str);
            let params = BrowseStoreParams {
                category: args
                    .get("category")
                    .and_then(Value::as_str)
                    .map(CoachCategory::parse),
                sort_by: args
                    .get("sort_by")
                    .and_then(Value::as_str)
                    .map_or(StoreSortOrder::Newest, StoreSortOrder::parse),
                limit: limit_arg(&args),
                cursor,
            };

            let page = browse_store(&repos, viewer_tenant, &params, &locale).await?;
            let payload = BrowseCoachStoreResult {
                count: page.coaches.len(),
                coaches: page.coaches.iter().map(project).collect(),
                has_more: page.has_more,
                next_cursor: page.next_cursor,
            };
            ok_typed("browse_coach_store", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Search published coaches in the Coach Store.
pub struct SearchCoachStoreTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SearchCoachStoreTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Text to match against a published coach's title, description or tags. \
                     Required."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Max results (1-100, default 20).".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["query".to_owned()]));

        answers_with::<Formatted<SearchCoachStoreResult>>(tool_definition(
            "search_coach_store",
            "Search the Coach Store for PUBLISHED coaches matching a phrase, e.g. 'ultra trail' or \
             'vegetarian nutrition'. Searches the whole marketplace, unlike `search_coaches`, \
             which searches only the athlete's own library. Install a result with \
             `install_coach_from_store`.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let Some(query) = args
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|q| !q.is_empty())
            else {
                return Ok(ToolResult::error(json!({
                    "error": "Missing required 'query' argument (the text to search the store for)."
                })));
            };

            let repos = context.resources.data().repos().coach_repos();
            let locale = athlete_locale(&context).await;
            let coaches = search_store(&repos, query, Some(limit_arg(&args)), &locale).await?;
            let rendered: Vec<StoreCoachEntry> = coaches.iter().map(project).collect();
            let payload = SearchCoachStoreResult {
                query: query.to_owned(),
                count: rendered.len(),
                coaches: rendered,
            };
            ok_typed("search_coach_store", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Install a published coach from the Coach Store.
pub struct InstallCoachFromStoreTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for InstallCoachFromStoreTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "UUID of the published coach to install, as returned by \
                     `browse_coach_store` or `search_coach_store`. Required."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<Formatted<InstallCoachFromStoreResult>>(tool_definition(
            "install_coach_from_store",
            "Install a published Coach Store coach into the athlete's own library, creating their \
             personal copy. Call it only once the athlete has asked for that specific coach — pass \
             the `id` from `browse_coach_store` or `search_coach_store`. After installing, \
             `activate_coach` makes it the coach that answers.",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let Some(coach_id) = args
                .get("coach_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|c| !c.is_empty())
            else {
                return Ok(ToolResult::error(json!({
                    "error": "Missing required 'coach_id' argument (the store agent's UUID)."
                })));
            };

            let user_id = context.user_id;
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let repos = context.resources.data().repos().coach_repos();
            let installed = install_store_coach(&repos, coach_id, user_id, tenant_id).await?;

            // `coach.installed` is emitted by `install_store_coach`, the one
            // install path this tool shares with the REST route and
            // `/discover install`, so it fires once per install on every surface.
            info!(
                user_id = %user_id,
                coach_id = %coach_id,
                "install_coach_from_store: coach installed from the store"
            );

            let payload = InstallCoachFromStoreResult {
                installed: true,
                // "agent library": main renamed the athlete-facing persona.
                message: format!(
                    "'{}' is now in your agent library. Activate it to start using it.",
                    installed.title
                ),
                coach: project(&installed),
            };
            ok_typed("install_coach_from_store", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// Guardian security classifications (see `crate::security`). The two reads echo
// coach-author-written titles, descriptions and sample prompts back into the
// LLM context, which is third-party text and therefore a taint source. Install
// creates a reversible copy, so it carries no label of its own.
crate::declare_security!(BrowseCoachStoreTool => UNTRUSTED_OUTPUT);
crate::declare_security!(SearchCoachStoreTool => UNTRUSTED_OUTPUT);
crate::declare_security!(InstallCoachFromStoreTool => empty);
