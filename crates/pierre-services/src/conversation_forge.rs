// ABOUTME: Forges a fresh chat conversation for an athlete — agent binding, counterpart title, channel stamp, guided flow
// ABOUTME: One ceremony for every caller: the messaging self-heal, /reset on any surface, and the title rule REST create shares

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Creating a conversation an athlete lands in.
//!
//! Two callers need the same five steps and must not drift apart: the
//! messaging ingress, when a session's `pierre_conversation_id` cannot be
//! reused, and `/reset`, when the athlete asks for a clean thread. Both want a
//! row bound to the right agent, titled after who the athlete is talking to,
//! stamped with the surface it was opened from, and — for an athlete who has
//! told us nothing yet — carrying the guided walk that stands in for the web
//! signup form.
//!
//! The title rule ([`counterpart_title`]) is public on its own because the
//! REST create route names a thread the same way without running the rest of
//! the ceremony: the room's name, else the bound agent's title, else a dated
//! stamp. The stored title is what every list row and thread header prints,
//! on both clients, so it has to be meaningful the moment the row exists.
//!
//! Everything here is a repository call, which is why it can live below both
//! callers rather than in either one.

use chrono::{DateTime, Utc};
use pierre_config::constants::usage_quotas::{
    DEFAULT_MAX_ACTIVE_CONVERSATIONS, UNLIMITED_CONVERSATIONS,
};
use pierre_config::environment::LlmProviderType;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoverageMap, GuidedFlow, OnboardingState, TenantId};
use pierre_database::repositories::{AgentsRepository, ChatRepository, TenantRepository};
use pierre_database::RepositoryRegistry;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup, ConfigLookupScope};
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent_selection::{record_agent_selection, AgentSelectionSource};
use crate::intake::is_outstanding;

/// Which agent the fresh conversation binds to.
#[derive(Debug, Clone, Copy)]
pub enum ForgeAgent<'a> {
    /// The athlete's tenant-level selected agent, else the tenant's first
    /// system agent. What a messaging DM uses: the session carries no agent
    /// of its own, and a DM with no agent at all is answered by the house
    /// prompt with nothing to name it after, so the room bind's fallback
    /// applies to a DM too.
    Selected,
    /// The athlete's tenant-level selected agent, or none. What a room
    /// member's row uses — the row a room session forges, whether the group
    /// is attached now or retrofitted on a later turn. The room's own agent
    /// lives on the group and answers there, and a system agent bound to the
    /// member's row would shadow the selected-agent rung every
    /// athlete-scoped read (`/plan`) resolves through.
    SelectedInRoom,
    /// The agent named here, carried from the thread being replaced. What
    /// `/reset` uses in the app, so an athlete resetting a conversation with
    /// one agent does not silently land on another.
    Explicit(Option<&'a str>),
}

/// Everything [`forge_conversation`] needs.
pub struct ForgeParams<'a> {
    /// The athlete the conversation belongs to.
    pub user_id: &'a str,
    /// Tenant that will own the `chat_conversations` row.
    ///
    /// A 1:1 thread files under the athlete's own tenant, a shared room under
    /// the channel's — pass the tenant the caller's *conversation* lives in,
    /// never the caller's own, or every later turn reads an empty thread.
    pub tenant_id: TenantId,
    /// What the row is called when it has neither a room nor an agent to be
    /// named after — the dated stamp in the surface's language. See
    /// [`counterpart_title`] for the rule that runs first.
    pub title_fallback: &'a str,
    /// Model to run the thread on. `None` falls back to `PIERRE_LLM_MODEL`.
    pub model: Option<&'a str>,
    /// Which agent to bind.
    pub agent: ForgeAgent<'a>,
    /// Group the thread belongs to, when it is a group thread.
    pub group_id: Option<&'a str>,
    /// Surface the conversation was opened from (`telegram`, `web`, …). The
    /// column defaults to `web`, so a thread forged from anywhere else must
    /// say so or it is badged wrong for the rest of its life.
    pub channel_type: &'a str,
    /// What to attribute the agent-usage bump to. `MessagingSession` for a
    /// channel thread, `ChatConversation` for one opened in the app — the
    /// counter measures conversations, not choices, so every forge records
    /// one.
    pub selection_source: AgentSelectionSource,
    /// Whether to offer the guided walk on the fresh row.
    ///
    /// True for a 1:1 thread, where the walk is how an athlete who never saw
    /// the web wizard tells us who they are. It still only fires for an
    /// athlete who has answered nothing anywhere — see [`start_guided_flow`].
    pub guided_flow: bool,
}

/// The `usage_quotas.max_active_conversations` key every quota read names.
pub const MAX_ACTIVE_CONVERSATIONS_KEY: &str = "usage_quotas.max_active_conversations";

/// The conversation cap in force for one athlete.
///
/// Resolved user → tenant → global, with the registered default when the
/// lookup fails or the key is unset. [`UNLIMITED_CONVERSATIONS`] (`0`) means
/// no cap.
pub async fn max_active_conversations(
    admin_config: &dyn AdminConfigLookup,
    user_id: &str,
    tenant_id: TenantId,
) -> i64 {
    admin_config
        .get_value(
            MAX_ACTIVE_CONVERSATIONS_KEY,
            ConfigLookupScope::user(user_id, &tenant_id.to_string()),
        )
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_MAX_ACTIVE_CONVERSATIONS)
}

/// Refuse one more thread for an athlete already at their cap.
///
/// The cap counts the conversations the athlete *owns* in the tenant, the
/// same rows the sidebar lets them delete, so the toast's advice is
/// actionable. It runs wherever the athlete asks for a fresh thread — the
/// REST create and `/reset` — but not where the messaging ingress forges one
/// to repair a session or answer a first contact: refusing there leaves the
/// channel dead, with no sidebar to delete from.
///
/// # Errors
///
/// [`AppError::quota_exceeded`] with `limit_type` `max_active_conversations`
/// when the count has reached the cap; the database error when the count
/// itself fails. An [`UNLIMITED_CONVERSATIONS`] cap never refuses.
pub async fn enforce_conversation_quota(
    repos: &RepositoryRegistry,
    admin_config: Option<&dyn AdminConfigLookup>,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<()> {
    // Degrade to the registered defaults when admin config is not wired
    // into the running server, never to no cap.
    let registered_defaults: &dyn AdminConfigLookup = default_admin_config();
    let admin_config = admin_config.unwrap_or(registered_defaults);
    let cap = max_active_conversations(admin_config, user_id, tenant_id).await;
    if cap == UNLIMITED_CONVERSATIONS {
        return Ok(());
    }
    let current = repos.chat.count_conversations(user_id, tenant_id).await?;
    (current < cap)
        .ok_or_else(|| AppError::quota_exceeded("max_active_conversations", current, cap, ""))
}

/// Create the conversation and return its id.
///
/// Best-effort for everything after the row exists: an agent-usage write, a
/// channel stamp or a guided-flow start that fails costs a nicety, never the
/// conversation the athlete is about to be dropped into.
///
/// # Errors
///
/// Returns [`AppError::config`] when no model is given and `PIERRE_LLM_MODEL`
/// is unset, and the database error when the row cannot be created.
pub async fn forge_conversation(
    repos: &RepositoryRegistry,
    params: ForgeParams<'_>,
) -> AppResult<String> {
    let ForgeParams {
        user_id,
        tenant_id,
        title_fallback,
        model,
        agent,
        group_id,
        channel_type,
        selection_source,
        guided_flow,
    } = params;

    let agent_id = match agent {
        ForgeAgent::Selected => selected_or_system_agent_for(repos, tenant_id, user_id).await,
        ForgeAgent::SelectedInRoom => selected_agent_id(repos, tenant_id, user_id).await,
        ForgeAgent::Explicit(id) => id.map(str::to_owned),
    };
    let title = counterpart_title(
        repos,
        tenant_id,
        user_id,
        group_id,
        agent_id.as_deref(),
        title_fallback,
    )
    .await;

    let model = match model {
        Some(m) => m.to_owned(),
        None => LlmProviderType::model_from_env().ok_or_else(|| {
            AppError::config("No model specified and PIERRE_LLM_MODEL environment variable not set")
        })?,
    };

    let conversation = repos
        .chat
        .create_conversation(
            user_id,
            tenant_id,
            &title,
            &model,
            agent_id.as_deref(),
            group_id,
        )
        .await?;
    let conversation_id = conversation.id;

    if let Some(agent_id) = agent_id.as_deref() {
        record_agent_usage(repos, agent_id, user_id, tenant_id, selection_source).await;
    }
    if guided_flow {
        start_guided_flow(repos, tenant_id, user_id, &conversation_id).await;
    }
    stamp_channel_origin(
        repos.chat.as_ref(),
        &conversation_id,
        user_id,
        tenant_id,
        channel_type,
    )
    .await;

    Ok(conversation_id)
}

/// The athlete's tenant-level selected agent, or `None`.
///
/// What a room row binds: the selection alone, never a system agent, so the
/// member's row shadows nothing (see [`ForgeAgent::SelectedInRoom`]). A
/// lookup failure reads as "no selection".
pub async fn selected_agent_id(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: &str,
) -> Option<String> {
    let parsed = Uuid::parse_str(user_id).ok()?;
    repos
        .tenants
        .get_selected_agent(tenant_id, parsed)
        .await
        .ok()?
}

/// The athlete's tenant-level selected agent, else the tenant's first system
/// agent, else `None`.
///
/// One answer for every place a 1:1 thread is bound to an agent without the
/// athlete naming one: the DM forge, the room bootstrap and the per-turn
/// rebind. A lookup failure reads as "no agent" — the thread is still usable
/// on the house prompt, the attribution panels simply skip it — and a tenant
/// with no system agent at all is the only way the answer stays empty.
pub async fn selected_or_system_agent(
    tenants: &dyn TenantRepository,
    agents: &dyn AgentsRepository,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Option<String> {
    if let Ok(Some(selected)) = tenants.get_selected_agent(tenant_id, user_id).await {
        return Some(selected);
    }
    agents
        .list_system_agents(tenant_id)
        .await
        .ok()?
        .first()
        .map(|agent| agent.id.to_string())
}

/// [`selected_or_system_agent`] over the registry, for a textual user id.
pub async fn selected_or_system_agent_for(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: &str,
) -> Option<String> {
    let parsed = Uuid::parse_str(user_id).ok()?;
    selected_or_system_agent(
        repos.tenants.as_ref(),
        repos.agents.as_ref(),
        tenant_id,
        parsed,
    )
    .await
}

/// The title a conversation is stored under: the room's name, else the bound
/// agent's title, else `fallback`.
///
/// A row with a room is the room, whoever answers in it; a 1:1 thread is the
/// agent the athlete talks to — on Telegram the DM with the bot carries the
/// bot's name on every thread, so the agent is the one fact that tells two
/// rows apart. Both clients print this value as-is, so the lookups happen
/// here, once, rather than in every list and header. A lookup that fails
/// falls through to the next tier rather than failing the forge.
pub async fn counterpart_title(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: &str,
    group_id: Option<&str>,
    agent_id: Option<&str>,
    fallback: &str,
) -> String {
    let group_name = match group_id {
        Some(id) => repos
            .groups
            .get_group(id, tenant_id)
            .await
            .ok()
            .flatten()
            .map(|group| group.name),
        None => None,
    };
    let agent_title = match (agent_id, Uuid::parse_str(user_id).ok()) {
        (Some(id), Some(user)) => agent_title(repos, id, user, tenant_id).await,
        _ => None,
    };
    forged_title(group_name.as_deref(), agent_title.as_deref(), fallback)
}

/// The bound agent's title, when the athlete can see the agent.
///
/// `get_by_id` answers for the athlete's own agents and every system agent,
/// which is the set a conversation can be bound to.
pub async fn agent_title(
    repos: &RepositoryRegistry,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    repos
        .agents
        .get_by_id(agent_id, user_id, tenant_id)
        .await
        .ok()
        .flatten()
        .map(|agent| agent.title)
}

/// The pure title rule behind [`counterpart_title`]: room, else agent, else
/// the fallback. A blank name at any tier counts as absent.
#[must_use]
pub fn forged_title(group_name: Option<&str>, agent_title: Option<&str>, fallback: &str) -> String {
    fn present(name: Option<&str>) -> Option<&str> {
        name.map(str::trim).filter(|name| !name.is_empty())
    }
    present(group_name)
        .or_else(|| present(agent_title))
        .unwrap_or(fallback)
        .to_owned()
}

/// Best-effort `agent_assignments.use_count++` through the shared recorder,
/// which also emits `agent.selected`.
async fn record_agent_usage(
    repos: &RepositoryRegistry,
    agent_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    source: AgentSelectionSource,
) {
    let Ok(user_uuid) = Uuid::parse_str(user_id) else {
        return;
    };
    if let Err(e) = record_agent_selection(
        repos.agents.as_ref(),
        agent_id,
        user_uuid,
        tenant_id,
        source,
    )
    .await
    {
        warn!(error = %e, agent_id, "Failed to record coach usage on a forged conversation");
    }
}

/// Record which surface opened the conversation.
async fn stamp_channel_origin(
    chat: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
) {
    if let Err(e) = chat
        .set_conversation_channel(conversation_id, user_id, tenant_id, channel_type)
        .await
    {
        warn!(error = %e, conversation_id, "Failed to stamp the conversation's channel_type");
    }
}

/// Put the intake, or failing that the pillar walk, on a fresh conversation.
///
/// The intake wins when it is outstanding, because it asks the two things
/// every later answer is read against. Neither fires for an athlete who has
/// already answered — on any surface, since coverage is shared.
async fn start_guided_flow(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: &str,
    conversation_id: &str,
) {
    if !maybe_start_intake(repos, user_id, conversation_id, tenant_id).await {
        maybe_start_pillar_walk(repos, tenant_id, user_id, conversation_id).await;
    }
}

/// Start the intake when the athlete still owes it. Returns whether it took.
async fn maybe_start_intake(
    repos: &RepositoryRegistry,
    user_id: &str,
    conversation_id: &str,
    tenant_id: TenantId,
) -> bool {
    let steps = match repos.user_onboarding.get_onboarding_steps(user_id).await {
        Ok(steps) => steps,
        Err(e) => {
            warn!(error = %e, "intake: could not read the onboarding steps; not starting");
            return false;
        }
    };
    if !is_outstanding(&steps) {
        return false;
    }
    activate(repos, conversation_id, tenant_id, GuidedFlow::Intake).await
}

/// Start the guided pillar walk when the athlete has told us nothing yet.
///
/// This is how a chat surface reaches parity with the web wizard: the wizard
/// asks on a form, the walk asks conversationally. Only fires on a genuinely
/// empty dossier — a returning athlete, or anyone who already answered on web,
/// is left alone, because coverage is shared across surfaces precisely so the
/// two never both ask.
///
/// Public because the intake hands over to it: an athlete who has just
/// finished the two intake questions is offered the walk on the same thread.
pub async fn maybe_start_pillar_walk(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: &str,
    conversation_id: &str,
) {
    let Ok(user_uuid) = Uuid::parse_str(user_id) else {
        return;
    };
    let Ok(dossier) = repos.dossier.compose_dossier(tenant_id, user_uuid).await else {
        return;
    };
    // Anything already captured means the walk has run, or web asked.
    if CoverageMap::from_dossier(&dossier).covered_count() > 0 {
        return;
    }
    activate(repos, conversation_id, tenant_id, GuidedFlow::Pillars).await;
}

/// Write a fresh guided-flow state onto the conversation. Returns whether the
/// row took it.
async fn activate(
    repos: &RepositoryRegistry,
    conversation_id: &str,
    tenant_id: TenantId,
    flow: GuidedFlow,
) -> bool {
    let json = OnboardingState::start_now_column(flow);
    match repos
        .chat
        .set_conversation_onboarding_state(conversation_id, Some(&json), tenant_id)
        .await
    {
        Ok(true) => {
            info!(conversation_id, flow = ?flow, "guided flow started on a fresh conversation");
            true
        }
        Ok(false) => {
            warn!(conversation_id, flow = ?flow, "guided-flow activation matched no conversation row");
            false
        }
        Err(e) => {
            warn!(error = %e, flow = ?flow, "guided flow failed to start");
            false
        }
    }
}

/// Point the messaging session that currently holds `previous_conversation_id`
/// at `conversation_id`, when one exists.
///
/// The in-app surfaces have no session, so this finding nothing is the normal
/// answer there, not a failure. On a messaging channel it is what makes the
/// rotation stick: the session binding is what the next inbound turn reads.
///
/// # Errors
///
/// Returns the database error when the session exists but cannot be repointed
/// — the athlete would otherwise be told they are on a fresh thread while the
/// channel keeps writing to the old one.
pub async fn repoint_messaging_session(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    previous_conversation_id: &str,
    conversation_id: &str,
) -> AppResult<bool> {
    let Some(session) = repos
        .messaging
        .get_session_by_pierre_conversation_id(tenant_id, previous_conversation_id)
        .await?
    else {
        return Ok(false);
    };
    let Some(session_id) = session["id"].as_str() else {
        warn!(
            previous_conversation_id,
            "messaging session row carries no id; leaving it pointed at the old conversation"
        );
        return Ok(false);
    };
    repos
        .messaging
        .set_session_conversation(session_id, conversation_id)
        .await?;
    Ok(true)
}

/// The title a conversation falls back to when it has neither a room nor an
/// agent to be named after.
///
/// The moment it started, in the reader's language: the localized prefix, the
/// short date, the 24-hour time — `Chat Sep 16 14:03`, `Discussion 16 sept.
/// 14:03`. The one dated form for every surface: the messaging forge, `/reset`
/// and the REST create route all stamp it, and no client invents its own. A
/// thread must not inherit the title of the thread it replaced: `/reset`
/// three times would otherwise leave three identically named rows in the list
/// with nothing to tell them apart.
#[must_use]
pub fn dated_title(prefix: &str, now: DateTime<Utc>) -> String {
    format!("{prefix} {}", now.format("%b %-d %H:%M"))
}
