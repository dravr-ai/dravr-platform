// ABOUTME: Admin API endpoints for viewing and editing system prompts via contremaitre
// ABOUTME: Supports listing prompts, reading content, and writing back to GitHub
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Contremaitre admin routes (mounted at `/api/admin/contremaitre/...`).
//!
//! Backed by the `pierre-contremaitre` registries carried on
//! [`crate::AdminApiContext`]. All endpoints are gated by the cookie-admin
//! middleware via [`crate::AdminRoutes::cookie_admin_routes`]; the
//! injected `ValidatedAdminToken` handles authorization here, except on the
//! two agent-prompt reads, which every console admin may make: the middleware's
//! role check is their gate. System prompts, the sync state and every write stay
//! behind `ViewConfiguration` and `ManageConfiguration`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::SUPPORTED_LOCALES;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use pierre_contremaitre::manifest::compute_sha256;
use pierre_contremaitre::registry::{PromptRegistry, PromptSource};
use pierre_contremaitre::sync::{
    full_sync, system_prompt_content_is_valid, ContremaitreRegistries,
};

use crate::context::AdminApiContext;

/// Build the contremaitre admin sub-router.
///
/// Mounted at the cookie-admin-protected `/api/admin/contremaitre/...`
/// prefix by [`crate::AdminRoutes::cookie_admin_routes`].
pub fn admin_routes(context: Arc<AdminApiContext>) -> Router {
    Router::new()
        .route(
            "/api/admin/contremaitre/prompts",
            get(handle_list_system_prompts),
        )
        .route(
            "/api/admin/contremaitre/prompts/{key}",
            get(handle_get_system_prompt).put(handle_update_system_prompt),
        )
        .route(
            "/api/admin/contremaitre/agent-prompts",
            get(handle_list_agent_prompts),
        )
        .route(
            "/api/admin/contremaitre/agent-prompts/{slug}",
            get(handle_get_agent_prompt),
        )
        .route("/api/admin/contremaitre/status", get(handle_status))
        .route("/api/admin/contremaitre/sync", post(handle_manual_sync))
        .with_state(context)
}

/// Response for listing all system prompts.
#[derive(Serialize)]
struct ListSystemPromptsResponse {
    /// Total number of system prompts
    total: usize,
    /// Prompt entries with metadata
    prompts: Vec<SystemPromptSummary>,
}

/// Summary of a single system prompt.
#[derive(Serialize)]
struct SystemPromptSummary {
    /// Prompt key (e.g., `"pierre_system"`)
    key: String,
    /// SHA-256 content hash
    sha256: String,
    /// Where this prompt was loaded from
    source: String,
    /// When this entry was last loaded
    loaded_at: String,
    /// Content length in characters
    content_length: usize,
}

/// Full content of a system prompt.
#[derive(Serialize)]
struct SystemPromptDetail {
    /// Prompt key
    key: String,
    /// Full prompt content
    content: String,
    /// SHA-256 content hash
    sha256: String,
    /// Where this prompt was loaded from
    source: String,
    /// When this entry was last loaded
    loaded_at: String,
}

/// Response for listing the agent prompts the registry holds.
#[derive(Serialize)]
struct ListAgentPromptsResponse {
    /// Number of agents
    total: usize,
    /// One entry per agent, sorted by slug
    prompts: Vec<AgentPromptSummary>,
}

/// Summary of one agent's prompts.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct AgentPromptSummary {
    /// Agent slug (e.g., `"endurance-agent"`)
    key: String,
    /// The locales the registry holds this agent in, sorted
    locales: Vec<String>,
}

/// The locale an agent prompt is requested in.
#[derive(Deserialize)]
struct AgentPromptQuery {
    /// One of `SUPPORTED_LOCALES`
    locale: String,
}

/// The agent prompt a turn in `requested_locale` is served.
#[derive(Debug, Serialize)]
struct AgentPromptDetail {
    /// Agent slug
    key: String,
    /// The locale the admin asked for
    requested_locale: String,
    /// The locale `content` is written in; differs from `requested_locale`
    /// when the default locale stands in for a missing translation
    locale: String,
    /// Full prompt content
    content: String,
    /// SHA-256 content hash
    sha256: String,
    /// Where this prompt was loaded from
    source: String,
    /// When this entry was last loaded
    loaded_at: String,
}

/// Request body for updating a system prompt.
#[derive(Deserialize)]
struct UpdateSystemPromptRequest {
    /// New prompt content
    content: String,
    /// Git commit message for the change
    commit_message: Option<String>,
}

/// Response after updating a system prompt.
#[derive(Serialize)]
struct UpdateSystemPromptResponse {
    /// Whether the update succeeded
    success: bool,
    /// New SHA-256 after update
    sha256: String,
    /// The manifest commit that completes the edit on GitHub; `None` when
    /// contremaitre is not configured and the edit lives in this process only
    commit_sha: Option<String>,
}

/// Contremaitre system status.
#[derive(Serialize)]
struct ContremaitreStatus {
    /// Whether contremaitre is configured
    configured: bool,
    /// Repository being synced from
    repo: Option<String>,
    /// Branch being synced from
    branch: Option<String>,
    /// Number of system prompts in the registry
    system_prompt_count: usize,
    /// Number of agent prompts in the registry, one per agent per locale
    agent_prompt_count: usize,
    /// Number of coaching-persona blocks in the registry
    persona_count: usize,
    /// How many of those three still hold their compiled-in seed copy
    compiled_in_count: usize,
    /// How many of those three a sync from contremaitre has replaced
    contremaitre_count: usize,
}

/// Manual sync result response.
#[derive(Serialize)]
struct SyncResponse {
    /// Whether the sync succeeded
    success: bool,
    /// Number of prompts synced
    synced: usize,
    /// Number of prompts skipped (unchanged)
    skipped: usize,
    /// Number that failed
    failed: usize,
}

/// GET /api/admin/contremaitre/prompts — list all system prompts with metadata.
async fn handle_list_system_prompts(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> Result<impl IntoResponse, AppError> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let entries = context.prompt_registry.list_system();
    let prompts: Vec<SystemPromptSummary> = entries
        .into_iter()
        .map(|(key, entry)| SystemPromptSummary {
            key,
            sha256: entry.sha256,
            source: match entry.source {
                PromptSource::CompiledIn => "compiled_in".to_owned(),
                PromptSource::Contremaitre => "contremaitre".to_owned(),
            },
            loaded_at: entry.loaded_at.to_rfc3339(),
            content_length: entry.content.len(),
        })
        .collect();

    let total = prompts.len();
    Ok(Json(ListSystemPromptsResponse { total, prompts }))
}

/// GET /api/admin/contremaitre/prompts/:key — get full content of a system prompt.
///
/// # Errors
///
/// Returns an error if the user is not an admin or the prompt key is not found.
async fn handle_get_system_prompt(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let entries = context.prompt_registry.list_system();
    let entry = entries
        .into_iter()
        .find(|(k, _)| k == &key)
        .map(|(_, e)| e)
        .ok_or_else(|| AppError::not_found(format!("System prompt '{key}'")))?;

    Ok(Json(SystemPromptDetail {
        key,
        content: entry.content,
        sha256: entry.sha256,
        source: match entry.source {
            PromptSource::CompiledIn => "compiled_in".to_owned(),
            PromptSource::Contremaitre => "contremaitre".to_owned(),
        },
        loaded_at: entry.loaded_at.to_rfc3339(),
    }))
}

/// Every agent the registry holds with its locales, keyed and sorted by slug.
fn agent_locales(registry: &PromptRegistry) -> BTreeMap<String, Vec<String>> {
    let mut agents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (slug, locale, _) in registry.list_agents() {
        agents.entry(slug).or_default().push(locale);
    }
    for locales in agents.values_mut() {
        locales.sort_unstable();
    }
    agents
}

/// The agent prompt a turn in `locale` is served, naming the locale it is in.
fn agent_prompt_detail(
    registry: &PromptRegistry,
    slug: &str,
    locale: &str,
) -> Result<AgentPromptDetail, AppError> {
    if !SUPPORTED_LOCALES.contains(&locale) {
        return Err(AppError::invalid_input(format!(
            "Unsupported locale: {locale}. Supported: {}",
            SUPPORTED_LOCALES.join(", ")
        )));
    }
    let (served, entry) = registry
        .agent_entry_for_locale(slug, locale)
        .ok_or_else(|| AppError::not_found(format!("Agent prompt '{slug}' in '{locale}'")))?;
    Ok(AgentPromptDetail {
        key: slug.to_owned(),
        requested_locale: locale.to_owned(),
        locale: served,
        content: entry.content,
        sha256: entry.sha256,
        source: match entry.source {
            PromptSource::CompiledIn => "compiled_in".to_owned(),
            PromptSource::Contremaitre => "contremaitre".to_owned(),
        },
        loaded_at: entry.loaded_at.to_rfc3339(),
    })
}

/// GET /api/admin/contremaitre/agent-prompts — list agents and their locales.
///
/// Open to every console admin; see the module docs.
async fn handle_list_agent_prompts(
    State(context): State<Arc<AdminApiContext>>,
) -> Result<impl IntoResponse, AppError> {
    let prompts: Vec<AgentPromptSummary> = agent_locales(&context.prompt_registry)
        .into_iter()
        .map(|(key, locales)| AgentPromptSummary { key, locales })
        .collect();
    let total = prompts.len();
    Ok(Json(ListAgentPromptsResponse { total, prompts }))
}

/// GET /api/admin/contremaitre/agent-prompts/:slug?locale= — one agent's prompt.
///
/// Open to every console admin; see the module docs.
///
/// # Errors
///
/// Returns an error if the locale is unsupported, or the registry holds the
/// agent in neither that locale nor the default one.
async fn handle_get_agent_prompt(
    State(context): State<Arc<AdminApiContext>>,
    Path(slug): Path<String>,
    Query(query): Query<AgentPromptQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(agent_prompt_detail(
        &context.prompt_registry,
        &slug,
        &query.locale,
    )?))
}

/// PUT /api/admin/contremaitre/prompts/:key — update a system prompt.
///
/// With contremaitre configured, the prompt and the manifest hash that
/// announces it are committed first, and only then is this instance's registry
/// updated: every other instance picks the edit up on its next sync poll, which
/// fetches a file whose manifest hash differs from the one it holds. A failed
/// commit changes nothing anywhere. Without contremaitre (local development)
/// the edit lives in this process's registry only, and `commit_sha` is `None`.
///
/// # Errors
///
/// Returns an error if the token lacks `ManageConfiguration`, if the content
/// misses a placeholder the prompt requires (every instance's sync would refuse
/// it), or if a GitHub read or commit fails.
async fn handle_update_system_prompt(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(key): Path<String>,
    Json(body): Json<UpdateSystemPromptRequest>,
) -> Result<impl IntoResponse, AppError> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let path = format!("prompts/system/{key}.md");
    if !system_prompt_content_is_valid(&key, &path, &body.content) {
        return Err(AppError::invalid_input(format!(
            "The {key} prompt is missing a placeholder it requires"
        )));
    }

    let commit_sha = match &context.contremaitre_config {
        Some(config) => {
            let default_message = format!("Update system prompt: {key}");
            let message = body.commit_message.as_deref().unwrap_or(&default_message);
            let sha = config
                .github_client()
                .commit_system_prompt(&key, &body.content, message)
                .await
                .inspect_err(|e| warn!(key, error = %e, "System prompt edit not committed"))?;
            info!(
                key,
                commit_sha = sha,
                "System prompt committed with its manifest hash"
            );
            Some(sha)
        }
        None => None,
    };

    let sha256 = compute_sha256(body.content.as_bytes());
    context
        .prompt_registry
        .update_system_prompt(&key, body.content, sha256.clone());
    info!(key, sha256, "System prompt updated in registry");

    Ok((
        StatusCode::OK,
        Json(UpdateSystemPromptResponse {
            success: true,
            sha256,
            commit_sha,
        }),
    ))
}

/// GET /api/admin/contremaitre/status — contremaitre system status.
///
/// # Errors
///
/// Returns an error if the user is not an admin.
async fn handle_status(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> Result<impl IntoResponse, AppError> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let stats = context.prompt_registry.stats();
    let (configured, repo, branch) = context
        .contremaitre_config
        .as_ref()
        .map_or((false, None, None), |config| {
            (true, Some(config.repo.clone()), Some(config.branch.clone()))
        });

    Ok(Json(ContremaitreStatus {
        configured,
        repo,
        branch,
        system_prompt_count: stats.system_count,
        agent_prompt_count: stats.agent_count,
        persona_count: stats.persona_count,
        compiled_in_count: stats.compiled_in_count,
        contremaitre_count: stats.contremaitre_count,
    }))
}

/// POST /api/admin/contremaitre/sync — trigger a manual full sync.
///
/// # Errors
///
/// Returns an error if the user is not an admin, contremaitre is not configured,
/// or the sync operation fails.
async fn handle_manual_sync(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> Result<impl IntoResponse, AppError> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let Some(config) = &context.contremaitre_config else {
        return Err(AppError::new(
            ErrorCode::ConfigMissing,
            "Contremaitre is not configured",
        ));
    };

    let store = config.store();
    let result = full_sync(
        ContremaitreRegistries {
            registry: &context.prompt_registry,
            tool_desc_registry: &context.tool_description_registry,
            evidence_registry: &context.evidence_registry,
            cageux_config_registry: &context.cageux_config_registry,
            messaging_strings_registry: &context.messaging_strings_registry,
            persona_contract_registry: &context.persona_contract_registry,
            training_catalogue_registry: &context.training_catalogue_registry,
        },
        store.as_ref(),
    )
    .await?;

    Ok((
        StatusCode::OK,
        Json(SyncResponse {
            success: true,
            synced: result.synced,
            skipped: result.skipped,
            failed: result.failed,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_with(locales: &[&str]) -> PromptRegistry {
        let registry = PromptRegistry::new();
        for locale in locales {
            registry.update_agent_prompt(
                "endurance-agent",
                locale,
                format!("text-{locale}"),
                format!("sha-{locale}"),
            );
        }
        registry
    }

    #[test]
    fn a_held_translation_is_shown_under_its_own_locale() {
        let registry = registry_with(&["en", "fr", "es"]);
        let detail = agent_prompt_detail(&registry, "endurance-agent", "es").unwrap();
        assert_eq!(detail.requested_locale, "es");
        assert_eq!(detail.locale, "es");
        assert_eq!(detail.content, "text-es");
    }

    #[test]
    fn a_missing_translation_says_which_locale_stands_in() {
        let registry = registry_with(&["en", "fr"]);
        let detail = agent_prompt_detail(&registry, "endurance-agent", "pt").unwrap();
        assert_eq!(detail.requested_locale, "pt");
        assert_eq!(detail.locale, "fr");
        assert_eq!(detail.content, "text-fr");
    }

    #[test]
    fn an_unsupported_locale_and_an_unknown_agent_are_refused() {
        let registry = registry_with(&["en", "fr"]);
        assert_eq!(
            agent_prompt_detail(&registry, "endurance-agent", "it")
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput,
        );
        assert_eq!(
            agent_prompt_detail(&registry, "nobody", "en")
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound,
        );
    }

    #[test]
    fn the_listing_sorts_agents_and_their_locales() {
        let registry = registry_with(&["pt", "en", "fr"]);
        registry.update_agent_prompt("activity-agent", "fr", "t".to_owned(), "s".to_owned());
        let listed: Vec<(String, Vec<String>)> = agent_locales(&registry).into_iter().collect();
        assert_eq!(
            listed,
            [
                ("activity-agent".to_owned(), vec!["fr".to_owned()]),
                (
                    "endurance-agent".to_owned(),
                    vec!["en".to_owned(), "fr".to_owned(), "pt".to_owned()],
                ),
            ]
        );
    }
}
