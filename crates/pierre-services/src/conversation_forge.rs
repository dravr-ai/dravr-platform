// ABOUTME: Forges a fresh chat conversation for an athlete — coach binding, channel stamp, guided flow
// ABOUTME: One ceremony for every caller: the messaging self-heal, and the /reset command on any surface

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Creating a conversation an athlete lands in.
//!
//! Two callers need the same five steps and must not drift apart: the
//! messaging ingress, when a session's `pierre_conversation_id` cannot be
//! reused, and `/reset`, when the athlete asks for a clean thread. Both want a
//! row bound to the right coach, stamped with the surface it was opened from,
//! and — for an athlete who has told us nothing yet — carrying the guided walk
//! that stands in for the web signup form.
//!
//! Everything here is a repository call, which is why it can live below both
//! callers rather than in either one.

use pierre_config::environment::LlmProviderType;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoverageMap, GuidedFlow, OnboardingState, TenantId};
use pierre_database::repositories::ChatRepository;
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

use crate::coach_selection::{record_coach_selection, CoachSelectionSource};
use crate::intake::is_outstanding;

/// Which coach the fresh conversation binds to.
#[derive(Debug, Clone, Copy)]
pub enum ForgeCoach<'a> {
    /// The athlete's tenant-level selected coach, or none when they have not
    /// picked one. What a messaging thread uses: the session carries no coach
    /// of its own, so the selection is the only answer available.
    Selected,
    /// The coach named here, carried from the thread being replaced. What
    /// `/reset` uses in the app, so an athlete resetting a conversation with
    /// one coach does not silently land on another.
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
    /// Title the conversation list will show.
    pub title: &'a str,
    /// Model to run the thread on. `None` falls back to `PIERRE_LLM_MODEL`.
    pub model: Option<&'a str>,
    /// Which coach to bind.
    pub coach: ForgeCoach<'a>,
    /// Group the thread belongs to, when it is a group thread.
    pub group_id: Option<&'a str>,
    /// Surface the conversation was opened from (`telegram`, `web`, …). The
    /// column defaults to `web`, so a thread forged from anywhere else must
    /// say so or it is badged wrong for the rest of its life.
    pub channel_type: &'a str,
    /// What to attribute the coach-usage bump to. `MessagingSession` for a
    /// channel thread, `ChatConversation` for one opened in the app — the
    /// counter measures conversations, not choices, so every forge records
    /// one.
    pub selection_source: CoachSelectionSource,
    /// Whether to offer the guided walk on the fresh row.
    ///
    /// True for a 1:1 thread, where the walk is how an athlete who never saw
    /// the web wizard tells us who they are. It still only fires for an
    /// athlete who has answered nothing anywhere — see [`start_guided_flow`].
    pub guided_flow: bool,
}

/// Create the conversation and return its id.
///
/// Best-effort for everything after the row exists: a coach-usage write, a
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
        title,
        model,
        coach,
        group_id,
        channel_type,
        selection_source,
        guided_flow,
    } = params;

    let coach_id = match coach {
        ForgeCoach::Selected => selected_coach_id(repos, tenant_id, user_id).await,
        ForgeCoach::Explicit(id) => id.map(str::to_owned),
    };

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
            title,
            &model,
            coach_id.as_deref(),
            group_id,
        )
        .await?;
    let conversation_id = conversation.id;

    if let Some(coach_id) = coach_id.as_deref() {
        record_coach_usage(repos, coach_id, user_id, tenant_id, selection_source).await;
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

/// The athlete's tenant-level selected coach, or `None`.
///
/// A lookup failure reads as "no coach": the thread is still usable, the
/// attribution panels simply skip it.
pub async fn selected_coach_id(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: &str,
) -> Option<String> {
    let parsed = Uuid::parse_str(user_id).ok()?;
    repos
        .tenants
        .get_selected_coach(tenant_id, parsed)
        .await
        .ok()?
}

/// Best-effort `coach_assignments.use_count++` through the shared recorder,
/// which also emits `coach.selected`.
async fn record_coach_usage(
    repos: &RepositoryRegistry,
    coach_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    source: CoachSelectionSource,
) {
    let Ok(user_uuid) = Uuid::parse_str(user_id) else {
        return;
    };
    if let Err(e) = record_coach_selection(
        repos.coaches.as_ref(),
        coach_id,
        user_uuid,
        tenant_id,
        source,
    )
    .await
    {
        warn!(error = %e, coach_id, "Failed to record coach usage on a forged conversation");
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

/// The title a freshly forged messaging conversation carries.
///
/// Named after the channel because that is all a messaging thread has: there
/// is no list to disambiguate it in and no title the athlete typed.
#[must_use]
pub fn messaging_title(channel_type: &str) -> String {
    format!("Messaging: {channel_type}")
}
