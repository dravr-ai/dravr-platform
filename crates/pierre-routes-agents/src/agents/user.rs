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
use pierre_cache::{CacheKey, CacheResource};
use pierre_config::agent_recommendations::AgentRecommendationConfig;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::agents::{
    AgentCategory, AgentHandle, AgentListItem, AgentPrerequisites, ListAgentsFilter,
    UpdateAgentRequest,
};
use pierre_core::models::{SportProfile, TenantId};
use pierre_llm::{ChatMessage, ChatRequest};
use pierre_middleware::AuthenticatedUser;
use pierre_providers::ai_scope;
use pierre_providers::registry::ProviderRegistry;
use pierre_runtime_context::{AgentsCtx, MiddlewareCtx};
use pierre_services::activity_sports::MessagingStringsRegistry;
use pierre_services::agent_generation::resolve_chat_provider;
use pierre_services::agent_selection::{record_agent_selection, AgentSelectionSource};
use pierre_services::agents as agents_service;
use pierre_services::locale::resolve_user_locale;
use pierre_services::team_rules::{self, pick_members, TeamPick, TeamRule, TeamSignals};
use pierre_tool_runtime::activity_fetch::fetch_recent_activities_all_providers;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::{debug, field, warn, Span};
use uuid::Uuid;

use super::proposal_lead::{
    best_declared_sport_agent, declared_sport_lead, proposal_order, put_lead_first,
};
use super::proposal_profile::{
    build_profile_view, pillar_context_prompt, proposal_goal, ProfileView,
};
use super::types::{
    validate_max_tool_iterations, AgentProposal, AgentProposalResponse, AgentResponse,
    ListAgentsQuery, ListAgentsResponse, MissingPrerequisite, ProposedAgent, ProposedTeam,
    RecordUsageResponse, SubmitForReviewResponse, TeamMember, UpdateAgentBody,
};

/// [`agents_service::user_sees_coach_tools`] for this route's context.
async fn sees_coach_tools<C: MiddlewareCtx>(ctx: &Arc<C>, user_id: Uuid) -> bool {
    agents_service::user_sees_coach_tools(
        MiddlewareCtx::repos(ctx.as_ref()).users.as_ref(),
        user_id,
    )
    .await
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

    let mut agents: Vec<AgentListItem> = manager.list(auth.user_id, tenant_id, &filter).await?;

    // Agent-facing builder personas are surfaced only to users in Agent mode;
    // athletes never see them in their library. Drop them before any scoring so
    // they can neither be recommended nor browsed.
    let users = MiddlewareCtx::repos(ctx.as_ref()).users.as_ref();
    agents_service::retain_visible_agents(users, auth.user_id, &mut agents, |item| &item.agent)
        .await;
    // The list reads in the athlete's locale, as the proposal does: Your team,
    // the chat header and Discover name a catalogue agent the way onboarding
    // offered it. Only this read view is translated; GET, which the editor
    // loads, keeps the stored text so an edit never saves a translation.
    let locale = resolve_user_locale(users, auth.user_id).await;
    MiddlewareCtx::repos(ctx.as_ref())
        .agents
        .apply_translations(&mut agents, &locale)
        .await?;
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
///
/// The scan reads through each provider's terms, so over an external transport
/// (an API key) it counts none of a provider whose data stays inside Dravr's
/// own surfaces. The cache is keyed by that class, so neither profile is ever
/// served to the other (carnet#724).
async fn load_sport_profile<C: ToolRuntime>(
    ctx: &Arc<C>,
    user_id: Uuid,
    tenant_id: TenantId,
    config: &AgentRecommendationConfig,
) -> Option<SportProfile> {
    let served_externally = ai_scope::exposure().is_some_and(|gate| gate.external);
    let cache_key = CacheKey::new(
        tenant_id,
        user_id,
        "coach_recs".to_owned(),
        CacheResource::Custom(
            if served_externally {
                "sport_profile:external"
            } else {
                "sport_profile"
            }
            .to_owned(),
        ),
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

    if let Err(e) = ToolRuntime::cache(ctx.as_ref())
        .set(
            &cache_key,
            &profile,
            Duration::from_secs(config.profile_ttl_secs),
        )
        .await
    {
        debug!(error = %e, "sport profile not cached; the next request recomputes it");
    }

    Some(profile)
}

/// Handle GET /api/agents/proposal - Onboarding agent proposal.
///
/// Drives the onboarding "Your team" step in one call: infers the user's
/// recent sport profile, deterministically prefilters the system-agent catalog
/// to an eligible candidate pool, then asks the LLM to re-rank that pool down
/// to the top [`max_recommended`](AgentRecommendationConfig::max_recommended)
/// with a one-line rationale each. The LLM step reads each candidate's "when
/// to use" text so a situational agent (e.g. a race-week taper builder) is
/// rejected for an athlete it does not fit. Beside the lead, the team rule
/// table ([`pierre_services::team_rules`]) picks the members; the same LLM
/// call writes their reasons. Any LLM failure falls back to deterministic
/// order and the rules' own reasons, so the proposal never hard-fails.
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
    let proposal = build_agent_proposal(&ctx, auth.user_id, tenant_id, &locale).await?;
    let response = AgentProposalResponse {
        profile: proposal.profile,
        agents: proposal.agents,
        team: proposal.team,
        goal: proposal.goal,
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
/// The team is `agents[0]` as lead plus the members the rule table picks
/// (published, bindable and provider-satisfiable catalogue agents only, never
/// the lead). Members are never re-rank candidates, so the model cannot make
/// one the lead; it writes their reasons in the same call. A cold start (no
/// usable profile) leads with the agent for the sport the athlete declared
/// when its provider prerequisite is met, otherwise with Fuelling, then
/// Recovery, then Mobility, and never with RED-S; it is re-ranked when the
/// athlete's about-you or pillar answers give the model something to read.
/// No proposal leads with a coach tool: the lead answers the person's own
/// training.
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
    user_id: Uuid,
    tenant_id: TenantId,
    locale: &str,
) -> Result<AgentProposal, AppError> {
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

    // Onboarding pillar context (North Star + covered pillars) enriches the
    // re-rank prompt, and the goal feeds the team rules. Best-effort — a
    // dossier compose failure never blocks the proposal; without one the match
    // falls back to sport-mix exactly as before.
    let dossier = MiddlewareCtx::repos(ctx.as_ref())
        .dossier
        .compose_dossier(
            tenant_id,
            user_id,
            ai_scope::readable_policy(),
            &ai_scope::admit_derived,
        )
        .await
        .ok();
    let pillar_context = dossier.as_ref().and_then(pillar_context_prompt);
    if let Some(context) = &pillar_context {
        profile_view.prompt_text = format!("{}\n\n{context}", profile_view.prompt_text);
    }

    // Score the full system-agent catalog and keep only eligible candidates.
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
    let sees_coach_tools = sees_coach_tools(ctx, user_id).await;
    // With no activity profile yet, the sport the athlete declared is what
    // makes a sport-specific agent eligible to lead (carnet#1151).
    let declared_sports = if sport_profile.is_none() {
        dossier
            .as_ref()
            .map(team_rules::declared_sports)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let eligible: Vec<(AgentListItem, f32)> = items
        .into_iter()
        .filter_map(|item| {
            if !sees_coach_tools && item.agent.is_coach_facing() {
                return None;
            }
            let profiled = agents_service::score_agent(
                &item.agent.prerequisites,
                sport_profile.as_ref(),
                &user_providers,
                &rec_config,
            );
            let recommendation = if profiled.eligible {
                profiled
            } else {
                agents_service::score_declared_sport(
                    &item.agent.prerequisites,
                    &declared_sports,
                    &user_providers,
                )
            };
            recommendation
                .eligible
                .then_some((item, recommendation.match_score))
        })
        .collect();

    // The rule table's members, resolved to published agents the athlete may
    // be given. They are resolved from the eligible list, so the same
    // coach-facing and prerequisite gates apply.
    let sleep_source = sleep_source_name(
        ToolRuntime::provider_registry(ctx.as_ref()),
        &user_providers,
    );
    let signals = TeamSignals::read(sport_profile.as_ref(), dossier.as_ref(), sleep_source);
    let picks = resolve_team_picks(
        ctx,
        &pick_members(&signals, rec_config.min_activities_for_sport),
        &eligible,
    )
    .await;

    let cold_start = sport_profile.is_none();
    let mut pool = eligible;
    pool.sort_by(|a, b| proposal_order(a, b, cold_start));
    pool.truncate(rec_config.rerank_pool_size);

    // Re-rank the candidate pool with the LLM when there is something to
    // reason about: a sport profile, or the athlete's own answers. A cold
    // start with neither takes the deterministic order.
    let (mut ranked, member_reasons) = rerank_candidates(
        ctx,
        &profile_view,
        pool,
        &picks,
        RerankOptions {
            max: rec_config.max_recommended,
            use_llm: sport_profile.is_some() || pillar_context.is_some(),
            cold_start,
        },
        locale,
    )
    .await;

    // The proposal reads in the athlete's locale end to end: the reasons
    // already do, and the agent titles take the same translation overlay the
    // store applies, so a French athlete is offered « Agent de trail », not
    // its English name.
    let agent_repo = &MiddlewareCtx::repos(ctx.as_ref()).agents;
    for (item, _, _) in &mut ranked {
        agent_repo
            .translate_agents(slice::from_mut(&mut item.agent), locale)
            .await?;
    }
    let mut picks = picks;
    for (item, _) in &mut picks {
        agent_repo
            .translate_agents(slice::from_mut(&mut item.agent), locale)
            .await?;
    }

    let team = ranked.first().map(|(lead, score, reason)| ProposedTeam {
        lead: ProposedAgent {
            agent: lead.clone().into(),
            match_score: *score,
            reason: reason.clone(),
        },
        members: team_members(
            picks,
            &lead.agent.id.to_string(),
            &member_reasons,
            sleep_source,
            ToolRuntime::messaging_strings_registry(ctx.as_ref()),
            locale,
        ),
    });
    let agents: Vec<ProposedAgent> = ranked
        .into_iter()
        .map(|(item, match_score, reason)| ProposedAgent {
            match_score,
            reason,
            agent: item.into(),
        })
        .collect();

    Ok(AgentProposal {
        profile: profile_view.summary,
        agents,
        team,
        goal: dossier.as_ref().and_then(proposal_goal),
    })
}

/// Display name of the first connected provider (by name, for a stable
/// answer) that sends sleep or recovery data.
fn sleep_source_name(
    registry: &ProviderRegistry,
    user_providers: &HashSet<String>,
) -> Option<&'static str> {
    let mut providers: Vec<&String> = user_providers.iter().collect();
    providers.sort();
    providers
        .into_iter()
        .find(|name| registry.supports_sleep(name) || registry.supports_recovery(name))
        .and_then(|name| registry.get_display_name(name))
}

/// The rule table's picks as agents the athlete may be given: the published
/// catalogue agent owning each handle, kept only when it is in `eligible`
/// (readable, not a coach tool for an athlete, prerequisites met). A handle
/// the catalogue does not publish is skipped.
async fn resolve_team_picks<C: MiddlewareCtx>(
    ctx: &Arc<C>,
    picks: &[TeamPick],
    eligible: &[(AgentListItem, f32)],
) -> Vec<(AgentListItem, TeamPick)> {
    let store = &MiddlewareCtx::repos(ctx.as_ref()).store_listings;
    let mut resolved = Vec::with_capacity(picks.len());
    for pick in picks {
        let Ok(handle) = AgentHandle::parse(pick.handle()) else {
            continue;
        };
        let published = match store.find_published_by_handle(&handle).await {
            Ok(Some(published)) => published.agent.id,
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %e, handle = pick.handle(), "team proposal: member lookup failed");
                continue;
            }
        };
        if let Some((item, _)) = eligible.iter().find(|(item, _)| item.agent.id == published) {
            resolved.push((item.clone(), *pick));
        }
    }
    resolved
}

/// The members beside `lead_id`, each with the LLM's reason or its rule's own.
fn team_members(
    picks: Vec<(AgentListItem, TeamPick)>,
    lead_id: &str,
    llm_reasons: &HashMap<String, String>,
    sleep_source: Option<&str>,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Vec<TeamMember> {
    picks
        .into_iter()
        .filter(|(item, _)| item.agent.id.to_string() != lead_id)
        .map(|(item, pick)| {
            let source = (pick.rule == TeamRule::SleepSource)
                .then(|| sleep_source.map(ToOwned::to_owned))
                .flatten();
            let reason = llm_reasons
                .get(&item.agent.id.to_string())
                .cloned()
                .unwrap_or_else(|| {
                    strings.render(
                        pick.rule.fallback_reason_key(),
                        locale,
                        &[source.as_deref().unwrap_or_default()],
                    )
                });
            TeamMember {
                agent: item.into(),
                rule: pick.rule,
                source,
                preselected: pick.preselected(),
                reason,
            }
        })
        .collect()
}

/// How [`rerank_candidates`] ranks.
struct RerankOptions {
    /// How many agents the proposal offers.
    max: usize,
    /// Whether to ask the LLM at all.
    use_llm: bool,
    /// No usable profile: RED-S never leads, an agent for the declared sport
    /// does when there is one (a coach tool never leads at all).
    cold_start: bool,
}

/// Re-rank `eligible` candidates into the final proposal list, and collect
/// the LLM's reasons for the team `members`.
///
/// When `use_llm` is true, asks the configured chat provider to pick the best
/// `max` agents with a rationale each, and to write one reason per member;
/// selections are applied in the model's order. Members are never offered to
/// the model as candidates, so it cannot make one the lead. Any LLM failure
/// (no provider, request error, unparseable output) yields no selections and
/// no reasons, and the function fills entirely from deterministic order. When
/// the LLM returns fewer than `max`, the remaining slots are filled
/// deterministically, members last. Returns `(agent, match_score, reason)`
/// tuples and the member reasons by id.
async fn rerank_candidates<C: AgentsCtx>(
    ctx: &Arc<C>,
    profile: &ProfileView,
    eligible: Vec<(AgentListItem, f32)>,
    members: &[(AgentListItem, TeamPick)],
    options: RerankOptions,
    locale: &str,
) -> (Vec<(AgentListItem, f32, String)>, HashMap<String, String>) {
    if eligible.is_empty() {
        return (Vec::new(), HashMap::new());
    }
    let member_ids: HashSet<String> = members
        .iter()
        .map(|(item, _)| item.agent.id.to_string())
        .collect();

    let candidates: Vec<agents_service::ProposalCandidate> = eligible
        .iter()
        .filter(|(item, _)| !member_ids.contains(&item.agent.id.to_string()))
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
    let member_context: Vec<team_rules::MemberContext> = members
        .iter()
        .map(|(item, pick)| team_rules::MemberContext {
            id: item.agent.id.to_string(),
            title: item.agent.title.clone(),
            why: team_rules::rule_context(pick.rule),
        })
        .collect();

    let (selections, member_reasons) = if options.use_llm && !candidates.is_empty() {
        llm_rerank_selections(
            ctx,
            &profile.prompt_text,
            &candidates,
            &member_context,
            options.max,
            locale,
        )
        .await
    } else {
        (Vec::new(), HashMap::new())
    };

    // Index candidates by id so selections pull out in the model's order.
    let mut by_id: HashMap<String, (AgentListItem, f32)> = eligible
        .into_iter()
        .map(|(item, score)| (item.agent.id.to_string(), (item, score)))
        .collect();

    let mut out: Vec<(AgentListItem, f32, String)> = Vec::with_capacity(options.max);
    for selection in selections {
        if let Some((item, score)) = by_id.remove(&selection.id) {
            out.push((item, score, selection.reason));
        }
    }

    // On a cold start a sport-specific agent is eligible only through the
    // sport the athlete declared, and it is the one that answers what they
    // train (carnet#1151): when the model offered none, the best one takes a
    // slot ahead of the deterministic fill.
    if options.cold_start && !out.iter().any(|(item, _, _)| declared_sport_lead(item)) {
        let best = best_declared_sport_agent(by_id.values(), &member_ids);
        if let Some((item, score)) = best.and_then(|id| by_id.remove(&id)) {
            out.truncate(options.max.saturating_sub(1));
            let reason = fallback_reason(&item, profile.primary_sport.as_deref(), locale);
            out.push((item, score, reason));
        }
    }

    // Deterministic fill for any unfilled slots (LLM returned too few, or off).
    if out.len() < options.max {
        let mut remaining: Vec<(AgentListItem, f32)> = by_id.into_values().collect();
        remaining.sort_by(|a, b| {
            let member = |item: &AgentListItem| member_ids.contains(&item.agent.id.to_string());
            member(&a.0)
                .cmp(&member(&b.0))
                .then_with(|| proposal_order(a, b, options.cold_start))
        });
        for (item, score) in remaining {
            if out.len() >= options.max {
                break;
            }
            let reason = fallback_reason(&item, profile.primary_sport.as_deref(), locale);
            out.push((item, score, reason));
        }
    }

    put_lead_first(&mut out, options.cold_start);

    (out, member_reasons)
}

/// Run the LLM re-rank call, returning ordered selections and the member
/// reasons, or empty on any failure (logged, never propagated — onboarding
/// falls back deterministically).
async fn llm_rerank_selections<C: AgentsCtx>(
    ctx: &Arc<C>,
    profile_prompt: &str,
    candidates: &[agents_service::ProposalCandidate],
    members: &[team_rules::MemberContext],
    max: usize,
    locale: &str,
) -> (
    Vec<agents_service::RankedSelection>,
    HashMap<String, String>,
) {
    let provider = match resolve_chat_provider(ctx.chat_provider(), ctx.llm_provider()) {
        Ok(provider) => provider,
        Err(e) => {
            warn!(error = %e, "coach proposal: no chat provider, using deterministic order");
            return (Vec::new(), HashMap::new());
        }
    };

    let valid_ids: HashSet<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let member_ids: HashSet<String> = members.iter().map(|m| m.id.clone()).collect();
    let user_prompt = format!(
        "{}{}",
        agents_service::build_rerank_user_prompt(profile_prompt, candidates, max, locale),
        team_rules::members_prompt_section(members),
    );
    let rerank_prompt = ctx.agent_rerank_prompt();
    let messages = vec![
        ChatMessage::system(rerank_prompt.trim()),
        ChatMessage::user(&user_prompt),
    ];

    match provider.complete(&ChatRequest::new(messages)).await {
        Ok(response) => (
            agents_service::parse_rerank_response(&response.content, &valid_ids, max),
            team_rules::parse_member_reasons(&response.content, &member_ids),
        ),
        Err(e) => {
            warn!(error = %e, "coach proposal: re-rank LLM call failed, using deterministic order");
            (Vec::new(), HashMap::new())
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

    // Enrich with the user's own row: team membership, favourite, counters.
    response.set_caller_state(manager.get_user_preferences(&id, auth.user_id).await?);

    Ok((StatusCode::OK, Json(response)).into_response())
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

    let mut response: AgentResponse = agent.into();
    response.set_caller_state(manager.get_user_preferences(&id, auth.user_id).await?);
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/agents/:id/fork - Fork an agent into the caller's own
/// editable copy.
///
/// An athlete edits an agent on their team — a catalogue agent they joined,
/// one assigned to them — by forking it first; the copy is theirs, carries
/// every runtime field of the origin, and takes over their membership (with
/// its counters and favourite) and their selection in this tenant. Answers
/// `201` with the copy. Forking again returns the copy already made, and
/// forking their own agent returns it unchanged. A coach-facing agent is
/// refused to an athlete exactly as binding it would be; an agent the caller
/// cannot see is `404`.
#[tracing::instrument(
    skip(ctx, auth),
    fields(
        route = "coach_fork",
        user_id = field::Empty,
        tenant_id = field::Empty,
    )
)]
pub(super) async fn handle_fork<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;
    let span = Span::current();
    span.record("user_id", field::display(&auth.user_id));
    span.record("tenant_id", field::display(&tenant_id));

    let manager = super::get_agents_manager(&ctx);
    agents_service::require_bindable_agent(
        manager,
        MiddlewareCtx::repos(ctx.as_ref()).users.as_ref(),
        &id,
        auth.user_id,
        tenant_id,
    )
    .await?;
    let copy = manager.fork_agent(&id, auth.user_id, tenant_id).await?;
    let copy_id = copy.id.to_string();

    let mut response: AgentResponse = copy.into();
    // The copy is on the caller's team, with the counters their row carried.
    response.set_caller_state(manager.get_user_preferences(&copy_id, auth.user_id).await?);

    Ok((StatusCode::CREATED, Json(response)).into_response())
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

/// Handle POST /api/agents/:id/submit - Submit one of the caller's agents to
/// the Store review queue.
///
/// Only the agent's author may submit it: an agent in the caller's tenant
/// that someone else wrote is refused with 403, one the caller cannot see at
/// all reads as 404. The listing lands in the admin review queue as
/// `pending_review`, where an admin approves or rejects it.
#[tracing::instrument(
    skip(ctx, auth),
    fields(
        route = "coach_submit_for_review",
        user_id = field::Empty,
        tenant_id = field::Empty,
    )
)]
pub(super) async fn handle_submit_for_review<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;
    let span = Span::current();
    span.record("user_id", field::display(&auth.user_id));
    span.record("tenant_id", field::display(&tenant_id));

    let agent = super::get_agents_manager(&ctx)
        .get_in_tenant(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach {id}")))?;
    if agent.user_id != auth.user_id {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only the agent's author can submit it to the Store",
        ));
    }

    let listing = super::get_store_manager(&ctx)
        .submit_for_review(&id, auth.user_id, tenant_id)
        .await?;

    let response = SubmitForReviewResponse {
        agent_id: id,
        publish_status: listing.publish_status,
        review_submitted_at: listing.review_submitted_at.map(|at| at.to_rfc3339()),
    };
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
