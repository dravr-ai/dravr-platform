use std::slice;
// ABOUTME: User-facing agent route handlers for listing, CRUD, favorites, and generation
// ABOUTME: Contains all non-admin agent endpoints that regular authenticated users access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration as ChronoDuration, Utc};
use pierre_agent_parser::{parse_agent_content, to_markdown};
use pierre_cache::{CacheKey, CacheResource};
use pierre_config::agent_recommendations::AgentRecommendationConfig;
use pierre_core::errors::AppError;
use pierre_core::models::agents::{
    AgentCategory, AgentListItem, AgentPrerequisites, CreateAgentRequest, ListAgentsFilter,
    UpdateAgentRequest,
};
use pierre_core::models::{CoachingPersona, SportProfile, TenantId};
use pierre_database::database::agents::compute_request_hash;
use pierre_llm::{ChatMessage, ChatRequest};
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{AgentsCtx, MiddlewareCtx};
use pierre_services::agent_generation::{agent_quota, resolve_chat_provider};
use pierre_services::agent_selection::{record_agent_selection, AgentSelectionSource};
use pierre_services::locale::resolve_user_locale;
use pierre_services::{agent_import, agents as agents_service, recipes as recipes_service};
use pierre_tool_runtime::activity_fetch::fetch_recent_activities_all_providers;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::{field, warn, Span};
use uuid::Uuid;

use super::proposal_profile::{build_profile_view, pillar_context_prompt, ProfileView};
use super::types::{
    validate_max_tool_iterations, AgentProposalResponse, AgentResponse, CreateAgentBody,
    ForkAgentResponse, HideAgentResponse, ImportAgentResponse, ImportFromUrlBody,
    ImportPreviewResponse, ListAgentsQuery, ListAgentsResponse, MissingPrerequisite,
    ParsedAgentFields, ProposedAgent, RecordUsageResponse, SearchAgentsQuery, SportProfileSummary,
    ToggleFavoriteResponse, UpdateAgentBody,
};

/// Whether the user may see coach-facing builder personas (agents tagged
/// [`Agent::COACH_TOOL_TAG`](pierre_core::models::agents::Agent::COACH_TOOL_TAG)).
///
/// Only users operating in the [`CoachingPersona::Coach`] mode — professional
/// coaches building plans for their athletes — get them. Athletes never see or
/// get recommended a coach-facing builder.
///
/// Fails closed: any user-lookup error resolves to `false`, so a transient
/// failure hides coach tools rather than leaking them to an athlete.
async fn user_sees_coach_tools<C: MiddlewareCtx>(ctx: &Arc<C>, user_id: Uuid) -> bool {
    MiddlewareCtx::repos(ctx.as_ref())
        .users
        .get_global(user_id)
        .await
        .ok()
        .flatten()
        .is_some_and(|user| user.coaching_persona == CoachingPersona::Coach)
}

/// Handle GET /api/agents - List agents for a user
pub(super) async fn handle_list<C: AgentsCtx + MiddlewareCtx + ToolRuntime>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);

    let filter = ListAgentsFilter {
        category: query.category.map(|c| AgentCategory::parse(&c)),
        favorites_only: query.favorites_only.unwrap_or(false),
        limit: query.limit,
        offset: query.offset,
        include_system: query.include_system.unwrap_or(true),
        include_hidden: query.include_hidden.unwrap_or(false),
    };

    let agents = manager.list(auth.user_id, tenant_id, &filter).await?;

    // Agent-facing builder personas are surfaced only to users in Agent mode;
    // athletes never see them in their library. Drop them before any scoring so
    // they can neither be recommended nor browsed.
    let agents: Vec<AgentListItem> = if user_sees_coach_tools(&ctx, auth.user_id).await {
        agents
    } else {
        agents
            .into_iter()
            .filter(|item| !item.agent.is_coach_facing())
            .collect()
    };
    // `total` is the user's global agent count (drives pagination) and stays
    // independent of the per-page `agents` view, which other query filters
    // (favorites_only / category) and the agent-facing filter narrow.
    let total = manager.count(auth.user_id, tenant_id).await?;

    let check_prereqs = query.check_prerequisites.unwrap_or(false);
    let personalize = query.personalize.unwrap_or(false);

    // Both prerequisite checking and personalization need the user's connected
    // providers. `ToolRuntime` and `MiddlewareCtx` both expose `repos()`, so
    // the call is disambiguated to the middleware view.
    let user_providers = if check_prereqs || personalize {
        MiddlewareCtx::repos(ctx.as_ref())
            .oauth_tokens
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

    // All recommendation tuning is env-driven (COACH_REC_*), so the curated
    // set can be retuned without a deploy.
    let rec_config = AgentRecommendationConfig::from_env();

    // Scan recent activities once for the whole list when personalizing.
    let sport_profile = if personalize {
        load_sport_profile(&ctx, auth.user_id, tenant_id, &rec_config).await
    } else {
        None
    };

    // First pass: build each response with prerequisite info, and capture the
    // recommendation score so we can cap the recommended set afterwards.
    let mut agents_with_prereqs: Vec<AgentResponse> = Vec::with_capacity(agents.len());
    let mut scores: Vec<(usize, f32, bool)> = Vec::new();

    for (index, item) in agents.into_iter().enumerate() {
        let prerequisites = item.agent.prerequisites.clone();
        let mut response: AgentResponse = item.into();

        if check_prereqs {
            let (met, missing) = check_prerequisites(&prerequisites, &user_providers);
            response.prerequisites_met = Some(met);
            response.missing_prerequisites = if missing.is_empty() {
                None
            } else {
                Some(missing)
            };
        }

        if personalize {
            let recommendation = agents_service::score_agent(
                &prerequisites,
                sport_profile.as_ref(),
                &user_providers,
                &rec_config,
            );
            response.match_score = Some(recommendation.match_score);
            scores.push((index, recommendation.match_score, recommendation.eligible));
        }

        agents_with_prereqs.push(response);
    }

    // Second pass: surface only the top `max_recommended` eligible agents by
    // score; the rest stay browsable with recommended=false.
    if personalize {
        scores.retain(|(_, _, eligible)| *eligible);
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        let recommended_indices: HashSet<usize> = scores
            .iter()
            .take(rec_config.max_recommended)
            .map(|(index, _, _)| *index)
            .collect();
        for (index, response) in agents_with_prereqs.iter_mut().enumerate() {
            response.recommended = Some(recommended_indices.contains(&index));
        }
    }

    let response = ListAgentsResponse {
        agents: agents_with_prereqs,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Load (or compute and cache) the user's recent sport profile for agent
/// personalization.
///
/// Returns `None` when the user has no connected provider or no activities in
/// the look-back window — callers treat that as cold start. The profile is
/// cached per (tenant, user) with a [`SPORT_PROFILE_TTL_SECS`] TTL so the
/// expensive provider scan doesn't run on every page load.
async fn load_sport_profile<C: ToolRuntime>(
    ctx: &Arc<C>,
    user_id: Uuid,
    tenant_id: TenantId,
    config: &AgentRecommendationConfig,
) -> Option<SportProfile> {
    let cache_key = CacheKey::new(
        tenant_id,
        user_id,
        "coach_recs".to_owned(),
        CacheResource::Custom("sport_profile".to_owned()),
    );

    if let Ok(Some(profile)) = ToolRuntime::cache(ctx.as_ref())
        .get::<SportProfile>(&cache_key)
        .await
    {
        return Some(profile);
    }

    // Two-step so the unsized coercion Arc<C> -> Arc<dyn ToolRuntime> happens
    // at the binding rather than inside Arc::clone's argument inference.
    let runtime_concrete: Arc<C> = Arc::clone(ctx);
    let runtime: Arc<dyn ToolRuntime> = runtime_concrete;
    let after_ts = (Utc::now() - ChronoDuration::days(i64::from(config.window_days))).timestamp();
    let activities = fetch_recent_activities_all_providers(
        &runtime,
        user_id,
        &tenant_id.to_string(),
        after_ts,
        config.activity_limit_per_provider,
    )
    .await;

    if activities.is_empty() {
        return None;
    }

    let profile = SportProfile::from_activities(&activities, config.window_days);

    let _ = ToolRuntime::cache(ctx.as_ref())
        .set(
            &cache_key,
            &profile,
            Duration::from_secs(config.profile_ttl_secs),
        )
        .await;

    Some(profile)
}

/// Handle GET /api/agents/proposal - Onboarding agent proposal.
///
/// Drives the post-onboarding "we analyzed your data → here are your coaches"
/// screen in one call: infers the user's recent sport profile, deterministically
/// prefilters the system-agent catalog to an eligible candidate pool, then asks
/// the LLM to re-rank that pool down to the top
/// [`max_recommended`](AgentRecommendationConfig::max_recommended) with a
/// one-line rationale each. The LLM step reads each candidate's "when to use"
/// text so a situational agent (e.g. a race-week taper builder) is rejected for
/// an athlete it does not fit. Any LLM failure falls back to deterministic
/// prefilter order, so the proposal never hard-fails.
pub(super) async fn handle_proposal<C: AgentsCtx + MiddlewareCtx + ToolRuntime>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;
    let locale = resolve_user_locale(
        MiddlewareCtx::repos(ctx.as_ref()).users.as_ref(),
        auth.user_id,
    )
    .await;
    let (profile, agents) = build_agent_proposal(&ctx, auth.user_id, tenant_id, &locale).await?;
    let response = AgentProposalResponse {
        profile,
        agents,
        metadata: super::build_metadata(),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Build the onboarding agent proposal for a user.
///
/// The shared core behind both the REST route and the messaging auto-send, so
/// every surface proposes identically. Infers the sport profile, deterministically
/// prefilters the system-agent catalog to an eligible pool, LLM-re-ranks it to the
/// top [`max_recommended`](AgentRecommendationConfig::max_recommended) with a
/// rationale each, and falls back to deterministic order on any LLM failure.
///
/// `locale` is the user's BCP-47 language code; it drives the language of each
/// agent's rationale (the LLM re-rank prompt and the deterministic fallback),
/// so a francophone user's proposal reads in French rather than English.
///
/// # Errors
///
/// Returns an error only if listing the agent catalog fails; profile inference
/// and the LLM step degrade gracefully rather than erroring.
pub async fn build_agent_proposal<C: AgentsCtx + MiddlewareCtx + ToolRuntime>(
    ctx: &Arc<C>,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
    locale: &str,
) -> Result<(SportProfileSummary, Vec<ProposedAgent>), AppError> {
    let manager = super::get_agents_manager(ctx);
    let rec_config = AgentRecommendationConfig::from_env();

    // Connected providers (lowercased names), needed for prefilter eligibility.
    let user_providers = MiddlewareCtx::repos(ctx.as_ref())
        .oauth_tokens
        .get_tokens(user_id, None)
        .await
        .map(|tokens| {
            tokens
                .iter()
                .map(|t| t.provider.to_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    // Recent sport profile (`None` ⇒ cold start: no provider or no activities).
    let sport_profile = load_sport_profile(ctx, user_id, tenant_id, &rec_config).await;
    let mut profile_view = build_profile_view(
        sport_profile.as_ref(),
        &user_providers,
        &rec_config,
        ToolRuntime::messaging_strings_registry(ctx.as_ref()),
        locale,
    );

    // Enrich the re-rank prompt with onboarding pillar context (North Star +
    // covered pillars). Graceful: when the user has no pillar context yet, the
    // match falls back to sport-mix exactly as before. Best-effort — a dossier
    // compose failure never blocks the proposal.
    if let Ok(dossier) = MiddlewareCtx::repos(ctx.as_ref())
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
    {
        if let Some(context) = pillar_context_prompt(&dossier) {
            profile_view.prompt_text = format!("{}\n\n{context}", profile_view.prompt_text);
        }
    }

    // Score the full system-agent catalog and keep only eligible candidates,
    // best deterministic score first, capped to the re-rank pool size.
    let filter = ListAgentsFilter {
        category: None,
        favorites_only: false,
        limit: None,
        offset: None,
        include_system: true,
        include_hidden: false,
    };
    let items = manager.list(user_id, tenant_id, &filter).await?;

    // Agent-facing builder personas are proposed only to users in Agent mode;
    // an athlete never gets an agent-facing builder recommended.
    let sees_coach_tools = user_sees_coach_tools(ctx, user_id).await;

    let mut eligible: Vec<(AgentListItem, f32)> = items
        .into_iter()
        .filter_map(|item| {
            if !sees_coach_tools && item.agent.is_coach_facing() {
                return None;
            }
            let recommendation = agents_service::score_agent(
                &item.agent.prerequisites,
                sport_profile.as_ref(),
                &user_providers,
                &rec_config,
            );
            recommendation
                .eligible
                .then_some((item, recommendation.match_score))
        })
        .collect();
    eligible.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.agent.id.cmp(&b.0.agent.id))
    });
    eligible.truncate(rec_config.rerank_pool_size);

    // Re-rank the candidate pool with the LLM. Cold start (no profile) skips the
    // LLM — there is nothing situational to reason about — and just takes the
    // deterministic top set.
    let ranked = rerank_candidates(
        ctx,
        &profile_view,
        eligible,
        rec_config.max_recommended,
        sport_profile.is_some(),
        locale,
    )
    .await;

    // The proposal reads in the athlete's locale end to end: the reasons
    // already do, and the agent titles take the same translation overlay the
    // store applies, so a French athlete is offered « Agent de trail », not
    // its English name.
    let mut ranked = ranked;
    let agent_repo = &MiddlewareCtx::repos(ctx.as_ref()).agents;
    for (item, _, _) in &mut ranked {
        agent_repo
            .translate_agents(slice::from_mut(&mut item.agent), locale)
            .await?;
    }

    let agents: Vec<ProposedAgent> = ranked
        .into_iter()
        .map(|(item, match_score, reason)| ProposedAgent {
            match_score,
            reason,
            agent: item.into(),
        })
        .collect();

    Ok((profile_view.summary, agents))
}

/// Re-rank `eligible` candidates into the final proposal list.
///
/// When `use_llm` is true, asks the configured chat provider to pick the best
/// `max` agents with a rationale each; selections are applied in the model's
/// order. Any LLM failure (no provider, request error, unparseable output)
/// yields no selections and the function fills entirely from deterministic
/// prefilter order. When the LLM returns fewer than `max`, the remaining slots
/// are filled deterministically. Returns `(agent, match_score, reason)` tuples.
async fn rerank_candidates<C: AgentsCtx>(
    ctx: &Arc<C>,
    profile: &ProfileView,
    eligible: Vec<(AgentListItem, f32)>,
    max: usize,
    use_llm: bool,
    locale: &str,
) -> Vec<(AgentListItem, f32, String)> {
    if eligible.is_empty() {
        return Vec::new();
    }

    let candidates: Vec<agents_service::ProposalCandidate> = eligible
        .iter()
        .map(|(item, score)| {
            // Rank on the explicit "when to use" guidance, then purpose, then
            // description — whichever the agent defines first.
            let blurb = item
                .agent
                .when_to_use
                .clone()
                .or_else(|| item.agent.purpose.clone())
                .or_else(|| item.agent.description.clone())
                .unwrap_or_default();
            agents_service::ProposalCandidate {
                id: item.agent.id.to_string(),
                title: item.agent.title.clone(),
                category: item.agent.category.as_str().to_owned(),
                tags: item.agent.tags.clone(),
                blurb,
                match_score: *score,
            }
        })
        .collect();
    let valid_ids: HashSet<String> = candidates.iter().map(|c| c.id.clone()).collect();

    let selections = if use_llm {
        llm_rerank_selections(
            ctx,
            &profile.prompt_text,
            &candidates,
            &valid_ids,
            max,
            locale,
        )
        .await
    } else {
        Vec::new()
    };

    // Index candidates by id so selections pull out in the model's order.
    let mut by_id: HashMap<String, (AgentListItem, f32)> = eligible
        .into_iter()
        .map(|(item, score)| (item.agent.id.to_string(), (item, score)))
        .collect();

    let mut out: Vec<(AgentListItem, f32, String)> = Vec::with_capacity(max);
    for selection in selections {
        if let Some((item, score)) = by_id.remove(&selection.id) {
            out.push((item, score, selection.reason));
        }
    }

    // Deterministic fill for any unfilled slots (LLM returned too few, or off).
    if out.len() < max {
        let mut remaining: Vec<(AgentListItem, f32)> = by_id.into_values().collect();
        remaining.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.0.agent.id.cmp(&b.0.agent.id))
        });
        for (item, score) in remaining {
            if out.len() >= max {
                break;
            }
            let reason = fallback_reason(&item, profile.primary_sport.as_deref(), locale);
            out.push((item, score, reason));
        }
    }

    out
}

/// Run the LLM re-rank call, returning ordered selections or an empty vec on any
/// failure (logged, never propagated — onboarding falls back deterministically).
async fn llm_rerank_selections<C: AgentsCtx>(
    ctx: &Arc<C>,
    profile_prompt: &str,
    candidates: &[agents_service::ProposalCandidate],
    valid_ids: &HashSet<String>,
    max: usize,
    locale: &str,
) -> Vec<agents_service::RankedSelection> {
    let provider = match resolve_chat_provider(ctx.chat_provider(), ctx.llm_provider()) {
        Ok(provider) => provider,
        Err(e) => {
            warn!(error = %e, "coach proposal: no chat provider, using deterministic order");
            return Vec::new();
        }
    };

    let user_prompt =
        agents_service::build_rerank_user_prompt(profile_prompt, candidates, max, locale);
    let messages = vec![
        ChatMessage::system(agents_service::AGENT_RERANK_SYSTEM_PROMPT),
        ChatMessage::user(&user_prompt),
    ];

    match provider.complete(&ChatRequest::new(messages)).await {
        Ok(response) => agents_service::parse_rerank_response(&response.content, valid_ids, max),
        Err(e) => {
            warn!(error = %e, "coach proposal: re-rank LLM call failed, using deterministic order");
            Vec::new()
        }
    }
}

/// Deterministic rationale used when the LLM does not select an agent,
/// localized to `locale` so the fallback path matches the user's language.
fn fallback_reason(item: &AgentListItem, primary_sport: Option<&str>, locale: &str) -> String {
    match primary_sport {
        Some(sport) if !item.agent.prerequisites.activity_types.is_empty() => {
            agents_service::fallback_reason_for_sport(sport, locale)
        }
        _ => agents_service::fallback_reason_generic(locale).to_owned(),
    }
}

/// Check if prerequisites are met given user's connected providers
///
/// Delegates to `services::agents::check_prerequisites` for the domain logic.
fn check_prerequisites(
    prerequisites: &AgentPrerequisites,
    user_providers: &HashSet<String>,
) -> (bool, Vec<MissingPrerequisite>) {
    let result = agents_service::check_prerequisites(prerequisites, user_providers);
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

/// Handle POST /api/agents - Create a new agent
pub(super) async fn handle_create<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Json(body): Json<CreateAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;
    validate_max_tool_iterations(body.max_tool_iterations)?;

    let manager = super::get_agents_manager(&ctx);

    // The same per-user cap `/agent create confirm` enforces, read through
    // the shared service so the two creation surfaces cannot drift.
    let quota = agent_quota(
        ctx.admin_config().as_deref(),
        manager,
        auth.user_id,
        tenant_id,
    )
    .await?;
    if quota.is_full() {
        return Err(AppError::quota_exceeded(
            "max_coaches_per_user",
            quota.current,
            quota.max,
            "",
        ));
    }

    let request: CreateAgentRequest = body.into();
    let agent = manager.create(auth.user_id, tenant_id, &request).await?;

    let response: AgentResponse = agent.into();
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle GET /api/agents/search - Search agents
pub(super) async fn handle_search<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(query): Query<SearchAgentsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agents = manager
        .search(auth.user_id, tenant_id, &query.q, query.limit, query.offset)
        .await?;

    let response = ListAgentsResponse {
        total: u32::try_from(agents.len()).unwrap_or(0),
        agents: agents.into_iter().map(Into::into).collect(),
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/agents/:id - Get a specific agent
pub(super) async fn handle_get<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agent = manager
        .get_by_id(&id, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    let mut response: AgentResponse = agent.into();

    // Enrich with user-specific preferences from agent_assignments
    let (is_favorite, use_count, last_used_at) =
        manager.get_user_preferences(&id, auth.user_id).await?;
    response.is_favorite = is_favorite;
    response.use_count = use_count;
    response.last_used_at = last_used_at.map(|dt| dt.to_rfc3339());

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/agents/:id/export - Export agent as markdown
pub(super) async fn handle_export<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agent = manager
        .get_by_id(&id, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    // Convert Agent to CoachDefinition for export.
    //
    // The markdown <-> definition conversion machinery lives in the `recipes`
    // service module and is deliberately reused here for the agents domain:
    // agents and recipes share the same on-disk markdown-with-frontmatter
    // representation, so the `agent_to_definition` / `generate_agent_filename`
    // helpers are domain-agnostic despite the module name.
    let definition = recipes_service::agent_to_definition(&agent);
    let markdown = to_markdown(&definition);

    // Generate filename from agent name/title (shared recipe markdown machinery).
    let filename = recipes_service::generate_agent_filename(&agent.title);

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

/// Handle POST /api/agents/import - Import agent from markdown
///
/// Parses markdown content, checks for duplicate content hashes, and
/// creates a new agent. Returns 409 Conflict if an agent with the same
/// content already exists for this user.
pub(super) async fn handle_import<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    body: String,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    // Parse the markdown content
    let definition = parse_agent_content(&body, None)
        .map_err(|e| AppError::invalid_input(format!("Invalid markdown format: {e}")))?;

    let warnings = agent_import::generate_import_warnings(&definition);
    let parsed_name = definition.frontmatter.name.clone();
    let token_count = definition.token_count;

    let request = agent_import::definition_to_create_request(&definition);

    // Check for duplicate using the same hash that create() will store
    let request_hash = compute_request_hash(&request);
    let manager = super::get_agents_manager(&ctx);
    if let Some(existing) = manager
        .find_by_content_hash(&request_hash, auth.user_id, tenant_id)
        .await?
    {
        return Err(AppError::already_exists(format!(
            "Coach with identical content (id: {})",
            existing.id
        )));
    }

    let agent = manager.create(auth.user_id, tenant_id, &request).await?;

    let response = ImportAgentResponse {
        agent: agent.into(),
        parsed_name,
        token_count,
        warnings,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle POST /api/agents/import/preview - Preview a markdown import without saving
///
/// Parses the markdown content and returns validation results, warnings,
/// and duplicate detection information without creating an agent.
pub(super) async fn handle_import_preview<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    body: String,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    match parse_agent_content(&body, None) {
        Ok(definition) => {
            let warnings = agent_import::generate_import_warnings(&definition);
            let token_count = definition.token_count;
            let request = agent_import::definition_to_create_request(&definition);
            let content_hash = compute_request_hash(&request);

            // Check for duplicate using the same hash that create() stores
            let manager = super::get_agents_manager(&ctx);
            let duplicate = manager
                .find_by_content_hash(&content_hash, auth.user_id, tenant_id)
                .await?;

            let parsed = ParsedAgentFields {
                name: definition.frontmatter.name,
                title: definition.frontmatter.title,
                category: definition.frontmatter.category.as_str().to_owned(),
                tags: definition.frontmatter.tags,
                purpose: definition.sections.purpose,
                has_instructions: !definition.sections.instructions.is_empty(),
                has_example_inputs: definition.sections.example_inputs.is_some(),
                has_example_outputs: definition.sections.example_outputs.is_some(),
                has_success_criteria: definition.sections.success_criteria.is_some(),
            };

            let response = ImportPreviewResponse {
                valid: true,
                parsed: Some(parsed),
                errors: Vec::new(),
                warnings,
                content_hash: Some(content_hash),
                duplicate_exists: duplicate.is_some(),
                duplicate_agent_id: duplicate.map(|c| c.id.to_string()),
                token_count: Some(token_count),
            };

            Ok((StatusCode::OK, Json(response)).into_response())
        }
        Err(e) => {
            let response = ImportPreviewResponse {
                valid: false,
                parsed: None,
                errors: vec![e.message],
                warnings: Vec::new(),
                content_hash: None,
                duplicate_exists: false,
                duplicate_agent_id: None,
                token_count: None,
            };

            Ok((StatusCode::OK, Json(response)).into_response())
        }
    }
}

/// Handle POST /api/agents/import/url - Import agent from a URL
///
/// Fetches markdown content from the given HTTPS URL with SSRF protection,
/// then either saves as a new agent or returns a preview depending on the
/// `save` parameter.
pub(super) async fn handle_import_from_url<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Json(body): Json<ImportFromUrlBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    // Fetch markdown from the URL (includes SSRF validation)
    let markdown = agent_import::fetch_markdown_from_url(&body.url).await?;

    // Parse the fetched markdown
    let definition = parse_agent_content(&markdown, Some(&body.url))
        .map_err(|e| AppError::invalid_input(format!("Invalid markdown format: {e}")))?;

    let warnings = agent_import::generate_import_warnings(&definition);
    let token_count = definition.token_count;
    let request = agent_import::definition_to_create_request(&definition);
    let content_hash = compute_request_hash(&request);

    let manager = super::get_agents_manager(&ctx);

    if !body.save {
        // Preview mode: return parsed fields without saving
        let duplicate = manager
            .find_by_content_hash(&content_hash, auth.user_id, tenant_id)
            .await?;

        let parsed = ParsedAgentFields {
            name: definition.frontmatter.name,
            title: definition.frontmatter.title,
            category: definition.frontmatter.category.as_str().to_owned(),
            tags: definition.frontmatter.tags,
            purpose: definition.sections.purpose,
            has_instructions: !definition.sections.instructions.is_empty(),
            has_example_inputs: definition.sections.example_inputs.is_some(),
            has_example_outputs: definition.sections.example_outputs.is_some(),
            has_success_criteria: definition.sections.success_criteria.is_some(),
        };

        let response = ImportPreviewResponse {
            valid: true,
            parsed: Some(parsed),
            errors: Vec::new(),
            warnings,
            content_hash: Some(content_hash),
            duplicate_exists: duplicate.is_some(),
            duplicate_agent_id: duplicate.map(|c| c.id.to_string()),
            token_count: Some(token_count),
        };

        return Ok((StatusCode::OK, Json(response)).into_response());
    }

    // Save mode: check for duplicates, then create
    if let Some(existing) = manager
        .find_by_content_hash(&content_hash, auth.user_id, tenant_id)
        .await?
    {
        return Err(AppError::already_exists(format!(
            "Coach with identical content (id: {})",
            existing.id
        )));
    }

    let parsed_name = definition.frontmatter.name.clone();
    let agent = manager.create(auth.user_id, tenant_id, &request).await?;

    let response = ImportAgentResponse {
        agent: agent.into(),
        parsed_name,
        token_count,
        warnings,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle PUT /api/agents/:id - Update an agent
pub(super) async fn handle_update<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    // Only a value the request actually assigns is range-checked; an absent
    // field and an explicit clear both mean "inherit" and carry no number.
    validate_max_tool_iterations(body.max_tool_iterations.assigned())?;

    let manager = super::get_agents_manager(&ctx);
    let request: UpdateAgentRequest = body.into();
    // The HTTP body carries no change-summary field, so the version snapshot
    // records an unsummarized edit.
    let agent = manager
        .update(&id, auth.user_id, tenant_id, &request, None)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    let response: AgentResponse = agent.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /api/agents/:id - Delete an agent
pub(super) async fn handle_delete<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let deleted = manager.delete(&id, auth.user_id, tenant_id).await?;

    if !deleted {
        return Err(AppError::not_found(format!("Coach {id}")));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// Handle POST /api/agents/:id/favorite - Toggle favorite status
pub(super) async fn handle_toggle_favorite<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let is_favorite = manager
        .toggle_favorite(&id, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;

    let response = ToggleFavoriteResponse { is_favorite };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/agents/:id/usage - Record agent usage
#[tracing::instrument(
    skip(ctx, auth),
    fields(
        route = "coach_record_usage",
        user_id = field::Empty,
        tenant_id = field::Empty,
    )
)]
pub(super) async fn handle_record_usage<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    // Record IDs on the span so this request's log lines carry the caller.
    let span = Span::current();
    span.record("user_id", field::display(&auth.user_id));
    span.record("tenant_id", field::display(&tenant_id));

    // `agent.selected` is emitted by `record_agent_selection`, which the web
    // chat, `/agent add` and messaging ingress also call — the event
    // follows the selection, not this transport.
    let manager = super::get_agents_manager(&ctx);
    let src = AgentSelectionSource::Rest;
    let success = record_agent_selection(manager, &id, auth.user_id, tenant_id, src).await?;

    if !success {
        return Err(AppError::not_found(format!("Coach {id}")));
    }

    let response = RecordUsageResponse { success };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/agents/:id/hide - Hide an agent from user's view
pub(super) async fn handle_hide_agent<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let success = manager.hide_agent(&id, auth.user_id, tenant_id).await?;

    let response = HideAgentResponse {
        success,
        is_hidden: success,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /api/agents/:id/hide - Show (unhide) an agent
pub(super) async fn handle_show_agent<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    // A caller with no active tenant is refused like every other agent route.
    // The hidden-set row is keyed per user, so the tenant is a gate here, not
    // a query key — see `AgentsRepository::show_agent`.
    super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let success = manager.show_agent(&id, auth.user_id).await?;

    let response = HideAgentResponse {
        success,
        is_hidden: false,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/agents/:id/fork - Fork a system agent to create a user copy
pub(super) async fn handle_fork<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let forked_agent = manager.fork_agent(&id, auth.user_id, tenant_id).await?;

    let response = ForkAgentResponse {
        agent: forked_agent.into(),
        source_agent_id: id,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle GET /api/agents/hidden - List hidden agents for user
pub(super) async fn handle_list_hidden<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agents = manager.list_hidden_agents(auth.user_id, tenant_id).await?;

    let response = ListAgentsResponse {
        total: u32::try_from(agents.len()).unwrap_or(0),
        agents: agents.into_iter().map(Into::into).collect(),
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
