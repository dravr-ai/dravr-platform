// ABOUTME: Starting a fixed-list guided walk on a conversation — the sequence /calibrate and /season share
// ABOUTME: Supersedes the previous run's window, records the new one, snapshots load, opens the flow, persists the opener
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppError;
use pierre_core::models::{
    AddMessageParams, GuidedFlow, GuidedWindow, LoadSnapshot, OnboardingState, Pillar, WalkAudience,
};
use pierre_messaging::commands::CommandResponse;
use pierre_services::recent_load::recent_load_snapshot;
use serde_json::Value;
use tracing::{info, warn};

use crate::PlatformCommandContext;

/// What distinguishes one fixed-list walk from another at start time. The
/// sequence itself — supersede, record, snapshot, activate, open — is the
/// same for every flow whose topics come from a list.
pub struct WalkSpec<'a> {
    /// The flow being started.
    pub flow: GuidedFlow,
    /// The command name, for log lines and error text.
    pub command: &'static str,
    /// Opener key for a walk started in a direct message.
    pub opener_private: &'a str,
    /// Opener key for a walk started in a shared room.
    pub opener_room: &'a str,
    /// Failure key when no conversation row matched.
    pub start_failed: &'a str,
    /// Format arguments for the private opener — the season walk quotes back
    /// what `/calibrate` recorded about the athlete's time.
    pub opener_args: &'a [&'a str],
}

/// The athlete's stored profile, read before anything is written.
///
/// A read that *fails* is not an athlete without a profile, and the two must
/// not collapse: `upsert_profile` replaces the whole document, so starting
/// from `None` after a failed read writes a bare window over the athlete's
/// nutrition and equipment blocks — the ones `compose_dossier` reads out of
/// this same blob — and supersedes nothing. The command reports the failure
/// instead, which leaves the stored profile exactly as it was.
///
/// # Errors
///
/// The repository error from the profile read.
pub async fn read_profile(
    ctx: &PlatformCommandContext,
    command: &str,
) -> Result<Option<Value>, AppError> {
    ctx.ctx
        .repos()
        .profiles
        .get_profile(ctx.user_id)
        .await
        .inspect_err(|e| {
            warn!(
                error = %e,
                user_id = %ctx.user_id,
                "{command} could not read the athlete's profile; walk not started"
            );
        })
}

/// Supersede the previous run's answers before capturing new ones.
///
/// The window is the only thing that identifies them: the fixed-list walks
/// cannot be matched by kind and pillar — calibration and the season walk
/// both land `goal`, `physiology` and `preference` facts in the training
/// pillar — and the extractor, not this code, chooses each fact's subject
/// and predicate, so there is no stable natural key to converge on either.
/// Scoping to the training pillar keeps a concurrently-running pillars
/// walk's other five pillars untouched; bounding by the previous run's
/// completion keeps the *other* fixed-list walk's answers, captured after
/// it, untouched too. A walk that never completed has no far bound and is
/// superseded up to now.
async fn supersede_previous_run(
    ctx: &PlatformCommandContext,
    profile: Option<&Value>,
    spec: &WalkSpec<'_>,
) -> Result<(), AppError> {
    let Some(previous) = GuidedWindow::last(profile, spec.flow) else {
        return Ok(());
    };
    let superseded = ctx
        .ctx
        .repos()
        .memory
        .expire_onboarding_facts(
            ctx.tenant_id,
            &ctx.user_id.to_string(),
            Some(Pillar::TrainingAndMovement),
            Some(previous.started_at),
            previous.completed_at,
            // Pillar-scoped re-screen: no per-question narrowing.
            None,
        )
        .await?;
    info!(
        user_id = %ctx.user_id,
        superseded,
        previous = %previous.started_at,
        "{} re-run superseded the previous walk",
        spec.command
    );
    Ok(())
}

/// Record this run's window for the *next* re-run to supersede, and snapshot
/// recent load once so every later turn reads the same figures.
///
/// A failed window write is not fatal to the walk — it costs the next re-run
/// its supersession, which shows up as duplicate answers rather than a broken
/// flow. A missing snapshot means no cached activity: calibration then asks
/// for the athlete's typical week instead of quoting a baseline, and the
/// season walk reads the athlete as single-sport.
async fn record_start_and_snapshot(
    ctx: &PlatformCommandContext,
    profile: Option<Value>,
    spec: &WalkSpec<'_>,
    started_at: &str,
) -> Option<LoadSnapshot> {
    let repos = ctx.ctx.repos();
    let merged = GuidedWindow::record_start(profile, spec.flow, started_at);
    if let Err(e) = repos.profiles.upsert_profile(ctx.user_id, merged).await {
        warn!(error = %e, user_id = %ctx.user_id, "failed to record the {} window", spec.command);
    }
    recent_load_snapshot(
        repos.activity_cache.as_ref(),
        ctx.user_id,
        &ctx.tenant_id,
    )
    .await
    .unwrap_or_else(|e| {
        warn!(error = %e, "activity cache unreadable; {} runs without a load snapshot", spec.command);
        None
    })
}

/// Persist the opener as an assistant message, for the same two reasons
/// `/pillars` does: the agent otherwise receives the athlete's first answer
/// with no question attached and improvises a reply to a question it cannot
/// see, and with no history row that answer is message #1, which arms the
/// first-turn startup prefetch.
async fn persist_opener(
    ctx: &PlatformCommandContext,
    conversation_id: &str,
    user: &str,
    msg: &str,
    command: &str,
) {
    let opener = AddMessageParams {
        tenant_id: ctx.conversation_tenant_id,
        conversation_id,
        user_id: user,
        role: "assistant",
        content: msg,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
        content_blocks: None,
    };
    if let Err(e) = ctx.ctx.repos().chat.add_message(&opener).await {
        warn!(error = %e, "failed to persist {command} opener as assistant message");
    }
}

/// Start the walk described by `spec` on the command's conversation.
///
/// Works in a direct message and in a shared room alike. Typing the command
/// in a room is the athlete's consent to a room-visible walk — the same
/// per-invocation grant `/plan share` established — and the walk binds to
/// them alone: the state carries their `subject_user_id`, so nobody else's
/// message advances it, and their agent follows along read-only. The walk
/// state and opener land on the conversation row's own tenant (the channel
/// tenant in a room), while the athlete-scoped writes — the supersession
/// window, the profile stamp, the load snapshot — stay under the athlete's
/// own tenant, which is also where the turn pipeline stamps every answer's
/// extraction.
///
/// # Errors
///
/// A missing conversation, a failed supersession, an unserializable state,
/// or a failed activation write.
pub async fn start_walk(
    ctx: &PlatformCommandContext,
    profile: Option<Value>,
    spec: WalkSpec<'_>,
) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let command = spec.command;

    let conversation_id = ctx.conversation_id.as_deref().ok_or_else(|| {
        AppError::invalid_input(format!("{command} needs an active conversation to run in"))
    })?;
    let user = ctx.user_id.to_string();
    let started_at = Utc::now().to_rfc3339();

    supersede_previous_run(ctx, profile.as_ref(), &spec).await?;
    let snapshot = record_start_and_snapshot(ctx, profile, &spec, &started_at).await;

    // The subject binding is what keeps a shared thread's walk the caller's
    // own; the audience records where they chose to run it, which is the
    // consent record and what filters room-unsafe topics.
    let audience = if ctx.is_direct_message {
        WalkAudience::Private
    } else {
        WalkAudience::Room
    };
    let state = OnboardingState::start(started_at, spec.flow)
        .with_snapshot(snapshot)
        .with_subject(user.clone())
        .with_audience(audience);
    let json = state
        .to_column()
        .map_err(|e| AppError::internal(format!("failed to serialize {command} state: {e}")))?;

    // The returned `bool` reports whether a row actually matched: a tenant
    // mismatch updates nothing, and answering with the opener anyway would
    // start a walk that no turn ever runs in. The conversation row lives
    // under the conversation's own tenant — the caller's in a DM, the channel
    // tenant in a shared room — and the update matches only there.
    let activated = ctx
        .ctx
        .repos()
        .chat
        .set_conversation_onboarding_state(conversation_id, Some(&json), ctx.conversation_tenant_id)
        .await?;
    if !activated {
        warn!(
            user_id = %ctx.user_id,
            conversation_id,
            "{command} matched no conversation row; walk not activated"
        );
        return Ok(CommandResponse::rich_text(reg.render(
            spec.start_failed,
            &ctx.locale,
            &[],
        )));
    }

    info!(
        user_id = %ctx.user_id,
        audience = ?audience,
        flow = ?spec.flow,
        "guided walk activated via {command}"
    );

    // The room opener says out loud what the athlete just chose: the
    // exchange is visible here, and it is theirs alone to answer.
    let msg = match audience {
        WalkAudience::Private => reg.render(spec.opener_private, &ctx.locale, spec.opener_args),
        WalkAudience::Room => reg.render(spec.opener_room, &ctx.locale, &[]),
    };
    persist_opener(ctx, conversation_id, &user, &msg, command).await;

    Ok(CommandResponse::rich_text(msg))
}
