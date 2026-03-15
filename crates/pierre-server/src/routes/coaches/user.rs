// ABOUTME: User-facing coach route handlers for listing, CRUD, favorites, and generation
// ABOUTME: Contains all non-admin coach endpoints that regular authenticated users access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;

use crate::{
    coaches::{parse_coach_content, to_markdown},
    errors::AppError,
    llm::{get_coach_generation_prompt, ChatMessage, ChatRequest},
    mcp::resources::ServerResources,
    services::{coaches as coaches_service, recipes as recipes_service},
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::models::coaches::{
    CoachCategory, CoachPrerequisites, CreateCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_database::database::{repositories::OAuthTokenRepository, ChatManager};
use std::sync::Arc;

use super::types::{
    CoachResponse, CreateCoachBody, ForkCoachResponse, GenerateCoachRequest, GenerateCoachResponse,
    GeneratedCoachData, HideCoachResponse, ImportCoachResponse, ListCoachesQuery,
    ListCoachesResponse, MissingPrerequisite, RecordUsageResponse, SearchCoachesQuery,
    ToggleFavoriteResponse, UpdateCoachBody,
};

/// Handle GET /api/coaches - List coaches for a user
pub(super) async fn handle_list(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Query(query): Query<ListCoachesQuery>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);

    let filter = ListCoachesFilter {
        category: query.category.map(|c| CoachCategory::parse(&c)),
        favorites_only: query.favorites_only.unwrap_or(false),
        limit: query.limit,
        offset: query.offset,
        include_system: query.include_system.unwrap_or(true),
        include_hidden: query.include_hidden.unwrap_or(false),
    };

    let coaches = manager.list(auth.user_id, tenant_id, &filter).await?;
    let total = manager.count(auth.user_id, tenant_id).await?;

    // Check prerequisites if requested
    let check_prereqs = query.check_prerequisites.unwrap_or(false);
    let user_providers = if check_prereqs {
        resources
            .database
            .get_tokens(auth.user_id, None)
            .await
            .map(|tokens| {
                tokens
                    .iter()
                    .map(|t| t.provider.to_lowercase())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default()
    } else {
        HashSet::new()
    };

    let coaches_with_prereqs: Vec<CoachResponse> = coaches
        .into_iter()
        .map(|item| {
            let prerequisites = item.coach.prerequisites.clone();
            let mut response: CoachResponse = item.into();

            if check_prereqs {
                let (met, missing) = check_prerequisites(&prerequisites, &user_providers);
                response.prerequisites_met = Some(met);
                response.missing_prerequisites = if missing.is_empty() {
                    None
                } else {
                    Some(missing)
                };
            }

            response
        })
        .collect();

    let response = ListCoachesResponse {
        coaches: coaches_with_prereqs,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Check if prerequisites are met given user's connected providers
///
/// Delegates to `services::coaches::check_prerequisites` for the domain logic.
fn check_prerequisites(
    prerequisites: &CoachPrerequisites,
    user_providers: &HashSet<String>,
) -> (bool, Vec<MissingPrerequisite>) {
    let result = coaches_service::check_prerequisites(prerequisites, user_providers);
    let missing = result
        .missing
        .into_iter()
        .map(|m| MissingPrerequisite {
            prerequisite_type: m.prerequisite_type,
            requirement: m.requirement,
            message: m.message,
        })
        .collect();
    (result.met, missing)
}

/// Handle POST /api/coaches - Create a new coach
pub(super) async fn handle_create(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(body): Json<CreateCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);

    // Enforce max_coaches_per_user limit from admin config
    if let Some(ref admin_config) = resources.admin_config {
        let max_coaches = admin_config
            .get_value(
                "usage_quotas.max_coaches_per_user",
                Some(&tenant_id.to_string()),
            )
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .unwrap_or(3);

        let current_count = i64::from(manager.count(auth.user_id, tenant_id).await?);
        if current_count >= max_coaches {
            return Err(AppError::quota_exceeded(
                "max_coaches_per_user",
                current_count,
                max_coaches,
                "",
            ));
        }
    }

    let request: CreateCoachRequest = body.into();
    let coach = manager.create(auth.user_id, tenant_id, &request).await?;

    let response: CoachResponse = coach.into();
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle GET /api/coaches/search - Search coaches
pub(super) async fn handle_search(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Query(query): Query<SearchCoachesQuery>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let coaches = manager
        .search(auth.user_id, tenant_id, &query.q, query.limit, query.offset)
        .await?;

    let response = ListCoachesResponse {
        total: u32::try_from(coaches.len()).unwrap_or(0),
        coaches: coaches.into_iter().map(Into::into).collect(),
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/coaches/:id - Get a specific coach
pub(super) async fn handle_get(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let coach = manager
        .get_by_id(&id, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    let mut response: CoachResponse = coach.into();

    // Enrich with user-specific preferences from coach_assignments
    let (is_favorite, _is_active, use_count, last_used_at) =
        manager.get_user_preferences(&id, auth.user_id).await?;
    response.is_favorite = is_favorite;
    response.use_count = use_count;
    response.last_used_at = last_used_at.map(|dt| dt.to_rfc3339());

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/coaches/:id/export - Export coach as markdown
pub(super) async fn handle_export(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let coach = manager
        .get_by_id(&id, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    // Convert Coach to CoachDefinition for export
    let definition = recipes_service::coach_to_definition(&coach);
    let markdown = to_markdown(&definition);

    // Generate filename from coach name/title
    let filename = recipes_service::generate_coach_filename(&coach.title);

    Ok((
        StatusCode::OK,
        [
            ("content-type", "text/markdown; charset=utf-8"),
            (
                "content-disposition",
                &format!("attachment; filename=\"{filename}\""),
            ),
        ],
        markdown,
    )
        .into_response())
}

/// Handle POST /api/coaches/import - Import coach from markdown
pub(super) async fn handle_import(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    // Parse the markdown content
    let definition = parse_coach_content(&body, None)
        .map_err(|e| AppError::invalid_input(format!("Invalid markdown format: {e}")))?;

    // Create coach from the parsed definition
    let request = CreateCoachRequest {
        title: definition.frontmatter.title,
        description: Some(definition.sections.purpose.clone()),
        system_prompt: definition.sections.instructions,
        category: definition.frontmatter.category,
        tags: definition.frontmatter.tags,
        sample_prompts: definition
            .sections
            .example_inputs
            .map(|inputs| {
                inputs
                    .lines()
                    .filter_map(|line| {
                        line.trim()
                            .strip_prefix('-')
                            .map(|s| s.trim().trim_matches('"').to_owned())
                    })
                    .collect()
            })
            .unwrap_or_default(),
        startup_query: None,
        data_requirements: None,
    };

    let manager = super::get_coaches_manager(&resources);
    let coach = manager.create(auth.user_id, tenant_id, &request).await?;

    let response = ImportCoachResponse {
        coach: coach.into(),
        parsed_name: definition.frontmatter.name,
        token_count: definition.token_count,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle POST /api/coaches/generate - Generate coach from conversation
///
/// Uses the LLM to analyze the last N messages of a conversation and
/// generate a coach profile with title, description, system prompt, and tags.
pub(super) async fn handle_generate(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(body): Json<GenerateCoachRequest>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    // Get chat manager to fetch conversation messages
    let pool = resources
        .database
        .sqlite_pool()
        .ok_or_else(|| AppError::internal("SQLite database required for coach generation"))?
        .clone();
    let chat_manager = ChatManager::new(pool);

    // Verify user owns the conversation (get_conversation returns None if not found or not owned)
    chat_manager
        .get_conversation(&body.conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation"))?;

    // Get conversation messages
    let messages = chat_manager
        .get_messages(&body.conversation_id, &auth.user_id.to_string())
        .await?;
    let total_messages = messages.len();

    if messages.is_empty() {
        return Err(AppError::invalid_input(
            "Cannot generate coach from empty conversation",
        ));
    }

    // Take the last N messages (or all if fewer)
    let messages_to_analyze: Vec<_> = messages
        .iter()
        .rev()
        .take(body.max_messages)
        .rev()
        .collect();
    let messages_analyzed = messages_to_analyze.len();

    // Build the conversation text for LLM analysis
    let conversation_text = messages_to_analyze
        .iter()
        .map(|m| format!("[{}]: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    // Build LLM request with generation prompt
    let system_prompt = get_coach_generation_prompt();
    let user_prompt = format!(
        "Analyze this fitness conversation and create a specialized coach profile.\n\n\
        Conversation (last {messages_analyzed} of {total_messages} messages):\n\n\
        {conversation_text}"
    );

    let llm_messages = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(&user_prompt),
    ];

    // Get LLM provider and generate
    let provider = super::super::create_chat_provider().await?;
    let request = ChatRequest::new(llm_messages);
    let response = provider.complete(&request).await?;

    if response.content.is_empty() {
        return Err(AppError::internal("LLM returned empty response"));
    }

    // Parse the JSON response from LLM
    let generated: GeneratedCoachData = serde_json::from_str(&response.content)
        .map_err(|e| AppError::internal(format!("Failed to parse LLM response as JSON: {e}")))?;

    let response = GenerateCoachResponse {
        title: generated.title,
        description: generated.description,
        system_prompt: generated.system_prompt,
        category: generated.category,
        tags: generated.tags,
        messages_analyzed,
        total_messages,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle PUT /api/coaches/:id - Update a coach
pub(super) async fn handle_update(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let request: UpdateCoachRequest = body.into();
    let coach = manager
        .update(&id, auth.user_id, tenant_id, &request)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    let response: CoachResponse = coach.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /api/coaches/:id - Delete a coach
pub(super) async fn handle_delete(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let deleted = manager.delete(&id, auth.user_id, tenant_id).await?;

    if !deleted {
        return Err(AppError::not_found(format!("Coach {id}")));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// Handle POST /api/coaches/:id/favorite - Toggle favorite status
pub(super) async fn handle_toggle_favorite(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let is_favorite = manager
        .toggle_favorite(&id, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    let response = ToggleFavoriteResponse { is_favorite };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/coaches/:id/usage - Record coach usage
pub(super) async fn handle_record_usage(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let success = manager.record_usage(&id, auth.user_id, tenant_id).await?;

    if !success {
        return Err(AppError::not_found(format!("Coach {id}")));
    }

    let response = RecordUsageResponse { success };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/coaches/:id/hide - Hide a coach from user's view
pub(super) async fn handle_hide_coach(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;

    let manager = super::get_coaches_manager(&resources);
    let success = manager.hide_coach(&id, auth.user_id).await?;

    let response = HideCoachResponse {
        success,
        is_hidden: success,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /api/coaches/:id/hide - Show (unhide) a coach
pub(super) async fn handle_show_coach(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;

    let manager = super::get_coaches_manager(&resources);
    let success = manager.show_coach(&id, auth.user_id).await?;

    let response = HideCoachResponse {
        success,
        is_hidden: false,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/coaches/:id/fork - Fork a system coach to create a user copy
pub(super) async fn handle_fork(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let forked_coach = manager.fork_coach(&id, auth.user_id, tenant_id).await?;

    let response = ForkCoachResponse {
        coach: forked_coach.into(),
        source_coach_id: id,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle GET /api/coaches/hidden - List hidden coaches for user
pub(super) async fn handle_list_hidden(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let coaches = manager.list_hidden_coaches(auth.user_id, tenant_id).await?;

    let response = ListCoachesResponse {
        total: u32::try_from(coaches.len()).unwrap_or(0),
        coaches: coaches.into_iter().map(Into::into).collect(),
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
