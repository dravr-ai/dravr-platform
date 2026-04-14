// ABOUTME: Admin API endpoints for viewing and editing system prompts via contremaitre
// ABOUTME: Supports listing prompts, reading content, and writing back to GitHub
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::errors::{AppError, ErrorCode};
use crate::mcp::resources::ServerResources;
use crate::middleware::{extract_auth_from_headers, require_admin};
use pierre_core::models::coaches::Coach;

use super::registry::PromptSource;

/// Mount contremaitre admin routes under /api/admin/contremaitre.
pub fn admin_routes(resources: Arc<ServerResources>) -> Router {
    Router::new()
        .route(
            "/admin/contremaitre/prompts",
            get(handle_list_system_prompts),
        )
        .route(
            "/admin/contremaitre/prompts/{key}",
            get(handle_get_system_prompt).put(handle_update_system_prompt),
        )
        .route("/admin/contremaitre/status", get(handle_status))
        .route("/admin/contremaitre/sync", post(handle_manual_sync))
        .route(
            "/admin/contremaitre/coaches/{id}/promote",
            post(handle_promote_coach),
        )
        .with_state(resources)
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
    /// Git commit SHA (if written to GitHub)
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
    /// Registry statistics
    system_prompt_count: usize,
    /// Number of coach prompts in registry
    coach_prompt_count: usize,
    /// Number loaded from compiled-in constants
    compiled_in_count: usize,
    /// Number loaded from contremaitre
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
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;

    let entries = resources.prompt_registry.list_system_prompts();
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
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;

    let entries = resources.prompt_registry.list_system_prompts();
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

/// PUT /api/admin/contremaitre/prompts/:key — update a system prompt.
///
/// Updates the in-memory registry immediately and asynchronously commits
/// the change to the contremaitre GitHub repository.
///
/// # Errors
///
/// Returns an error if the user is not an admin.
async fn handle_update_system_prompt(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<UpdateSystemPromptRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;

    // Compute new hash and update registry immediately (optimistic)
    let sha256 = super::manifest::compute_sha256(body.content.as_bytes());
    resources
        .prompt_registry
        .update_system_prompt(&key, body.content.clone(), sha256.clone());

    info!(key, sha256, "System prompt updated in registry");

    // Asynchronously commit to GitHub if contremaitre is configured
    let commit_sha = commit_to_github(&resources, &key, &body).await;

    Ok((
        StatusCode::OK,
        Json(UpdateSystemPromptResponse {
            success: true,
            sha256,
            commit_sha,
        }),
    ))
}

/// Fetch the current Git blob SHA for a file in the contremaitre repo.
async fn fetch_file_sha(
    client: &super::github::GitHubContentsClient,
    key: &str,
    path: &str,
) -> Option<String> {
    match client.read_file(path).await {
        Ok(file) => Some(file.sha),
        Err(e) => {
            warn!(key, error = %e, "Failed to read file SHA from GitHub");
            None
        }
    }
}

/// Commit the updated prompt content to GitHub and log the outcome.
async fn commit_updated_prompt(
    client: &super::github::GitHubContentsClient,
    key: &str,
    path: &str,
    content: &str,
    message: &str,
    file_sha: &str,
) -> Option<String> {
    match client
        .commit_file(path, content, message, Some(file_sha))
        .await
    {
        Ok(sha) => {
            info!(key, commit_sha = sha, "System prompt committed to GitHub");
            Some(sha)
        }
        Err(e) => {
            warn!(key, error = %e, "Failed to commit system prompt to GitHub");
            None
        }
    }
}

/// Helper function to commit an updated prompt to GitHub.
async fn commit_to_github(
    resources: &Arc<ServerResources>,
    key: &str,
    body: &UpdateSystemPromptRequest,
) -> Option<String> {
    let config = resources.contremaitre_config.as_ref()?;
    let client = config.github_client();
    let path = format!("prompts/system/{key}.md");
    let default_message = format!("Update system prompt: {key}");
    let message = body.commit_message.as_deref().unwrap_or(&default_message);

    let file_sha = fetch_file_sha(&client, key, &path).await?;
    commit_updated_prompt(&client, key, &path, &body.content, message, &file_sha).await
}

/// GET /api/admin/contremaitre/status — contremaitre system status.
///
/// # Errors
///
/// Returns an error if the user is not an admin.
async fn handle_status(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;

    let stats = resources.prompt_registry.stats();
    let (configured, repo, branch) = resources
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
        coach_prompt_count: stats.coach_count,
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
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;

    let Some(config) = &resources.contremaitre_config else {
        return Err(AppError::new(
            ErrorCode::ConfigMissing,
            "Contremaitre is not configured",
        ));
    };

    let client = config.github_client();
    let result = super::sync::full_sync(
        &resources.prompt_registry,
        &resources.tool_description_registry,
        &resources.evidence_registry,
        &client,
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

/// Response after promoting a coach.
#[derive(Serialize)]
struct PromoteCoachResponse {
    /// Whether the promotion succeeded
    success: bool,
    /// Path of the file in the contremaitre repo
    path: String,
    /// Git commit SHA
    commit_sha: Option<String>,
}

/// POST /api/admin/contremaitre/coaches/{id}/promote — promote a coach to contremaitre.
///
/// Reads the coach from the database, generates a markdown file, and commits it
/// to the contremaitre GitHub repository. Updates the coach's `source` to "contremaitre".
///
/// # Errors
///
/// Returns an error if the coach is not found, the user is not an admin,
/// or the GitHub commit fails.
async fn handle_promote_coach(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(coach_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;

    let Some(config) = &resources.contremaitre_config else {
        return Err(AppError::new(
            ErrorCode::ConfigMissing,
            "Contremaitre is not configured",
        ));
    };

    // Fetch the coach from the database
    let coach = resources
        .repos
        .coaches
        .get_system_coach_any_tenant(&coach_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

    // Build markdown content from coach fields
    let markdown = build_coach_markdown(&coach);

    // Determine the file path based on category
    let slug = coach
        .title
        .to_lowercase()
        .replace(' ', "-")
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "");
    let category = coach.category.as_str().to_lowercase();
    let path = format!("prompts/coaches/{category}/{slug}.md");
    let message = format!("Promote coach: {}", coach.title);

    // Commit to GitHub (create new file)
    let client = config.github_client();
    let commit_sha = match client.commit_file(&path, &markdown, &message, None).await {
        Ok(sha) => {
            info!(
                coach_id,
                path,
                commit_sha = sha,
                "Coach promoted to contremaitre"
            );
            Some(sha)
        }
        Err(e) => {
            warn!(coach_id, error = %e, "Failed to promote coach to contremaitre");
            return Err(AppError::new(
                ErrorCode::ExternalServiceError,
                format!("Failed to commit to GitHub: {e}"),
            ));
        }
    };

    // The coach source column will be updated to "contremaitre" on the next
    // webhook sync when the contremaitre repo processes the new file.

    Ok((
        StatusCode::OK,
        Json(PromoteCoachResponse {
            success: true,
            path,
            commit_sha,
        }),
    ))
}

/// Build markdown content from a `Coach` model for the contremaitre repo.
fn build_coach_markdown(coach: &Coach) -> String {
    use std::fmt::Write;

    let mut md = String::new();

    // YAML frontmatter
    md.push_str("---\n");
    let _ = writeln!(md, "name: {}", coach.title.to_lowercase().replace(' ', "-"));
    let _ = writeln!(md, "title: {}", coach.title);
    let _ = writeln!(md, "category: {}", coach.category.as_str());
    if !coach.tags.is_empty() {
        let _ = writeln!(md, "tags: [{}]", coach.tags.join(", "));
    }
    let _ = writeln!(md, "visibility: {}", coach.visibility.as_str());
    md.push_str("---\n\n");

    // Sections
    if let Some(purpose) = &coach.purpose {
        let _ = writeln!(md, "## Purpose\n\n{purpose}\n");
    }

    // Instructions (or fall back to system_prompt)
    let instructions = coach
        .instructions
        .as_deref()
        .unwrap_or(&coach.system_prompt);
    let _ = writeln!(md, "## Instructions\n\n{instructions}\n");

    if let Some(inputs) = &coach.example_inputs {
        let _ = writeln!(md, "## Example Inputs\n\n{inputs}\n");
    }
    if let Some(outputs) = &coach.example_outputs {
        let _ = writeln!(md, "## Example Outputs\n\n{outputs}\n");
    }
    if let Some(criteria) = &coach.success_criteria {
        let _ = writeln!(md, "## Success Criteria\n\n{criteria}\n");
    }

    md
}
