// ABOUTME: The turn subject's own athlete context — freshness, OKF bundle, playbooks, notes, followups, commitments
// ABOUTME: One gate for prompt assembly: a coach's seat in their group skips all of it (carnet#741)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Prompt stages 7d–7f.1: the turn subject's own athlete context.
//!
//! Everything here is read from the sender's own records. Grouped so the one
//! decision that the sender is not the subject (the coach's seat, see
//! [`super::group_subject`]) skips them together rather than stage by stage.

use pierre_services::memory_facts::SentenceRenderer;
use uuid::Uuid;

use super::super::surface_profile::SurfaceProfile;
use super::super::turn::TurnInput;
use super::commitments::inject_commitments;
use super::followups::inject_pending_followups;
use super::memory::{inject_agent_notes, inject_okf_bundle, inject_playbooks};
use super::refresh::inject_refresh_context;
use crate::ChatPipelineContext;

/// What the subject-context stages read.
pub struct SubjectContextInputs<'a> {
    /// Shared pipeline context.
    pub ctx: &'a ChatPipelineContext,
    /// The turn; its sender is the subject.
    pub input: &'a TurnInput,
    /// The turn's surface, for the OKF renderer's locale.
    pub profile: &'a SurfaceProfile,
    /// The sender's id, parsed.
    pub user_uuid: Uuid,
    /// The agent answering this turn, whose playbooks, notes and followups apply.
    pub turn_agent_id: Option<&'a str>,
    /// The sender's IANA timezone, when reported.
    pub user_timezone: Option<&'a str>,
}

/// Append the subject's own athlete context to `base_prompt`.
///
/// Returns the prompt with the ids of the followups it surfaced, which are
/// marked delivered after the turn succeeds.
pub async fn inject_subject_context(
    inputs: SubjectContextInputs<'_>,
    base_prompt: String,
) -> (String, Vec<String>) {
    let SubjectContextInputs {
        ctx,
        input,
        profile,
        user_uuid,
        turn_agent_id,
        user_timezone,
    } = inputs;

    // Stage 7d: Trigger background provider refresh and append freshness hint.
    // How stale the athlete's provider data is has the same bearing on the
    // answer on every surface, so no surface suppresses it.
    let auth_repos = ctx.repos.auth_repos();
    let base_prompt = inject_refresh_context(
        super::refresh::RefreshDeps {
            auth_repos: &auth_repos,
            activity_cache: ctx.repos.activity_cache.clone(),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: &ctx.sync_orchestrator,
            #[cfg(feature = "health-sync")]
            sse_manager: &ctx.sse_manager,
        },
        &input.user_id,
        input.tool_tenant_id,
        base_prompt,
    )
    .await;

    // Stage 7e: Render the per-user OKF context bundle (North Star + pillar +
    // medical facts) from the read-time Dossier into the prompt. Single
    // fact->prompt surface; user-wide (agent-agnostic) facts.
    let base_prompt = inject_okf_bundle(
        ctx.repos.dossier.as_ref(),
        input.conversation_tenant_id,
        user_uuid,
        base_prompt,
        SentenceRenderer::new(&ctx.messaging_strings_registry, &profile.locale),
    )
    .await;

    // Stage 7e.2: Inject the athlete's proven coaching playbooks (learned from
    // their own outcomes) so the agent prefers what has worked for them. Scoped
    // to the TOOL tenant — where the activity data and playbooks live.
    let playbook_tenant = input.tool_tenant_id.to_string();
    let base_prompt = inject_playbooks(
        ctx.repos.playbooks.as_ref(),
        ctx.repos.activity_cache.as_ref(),
        &playbook_tenant,
        &input.user_id,
        turn_agent_id,
        base_prompt,
    )
    .await;

    // Stage 7e.3: The notes the answering agent wrote about this athlete with
    // `agent_note_add`, newest first, suppressed ones left out. Scoped to the
    // TOOL tenant — the tenant the note tool writes under.
    let base_prompt = inject_agent_notes(
        ctx.repos.memory.as_ref(),
        input.tool_tenant_id,
        &input.user_id,
        turn_agent_id,
        base_prompt,
    )
    .await;

    // Stage 7f: Render pending agent followups. Surfaced IDs are marked
    // delivered after the turn succeeds.
    let (base_prompt, pending_followup_ids) = inject_pending_followups(
        &ctx.data,
        input.conversation_tenant_id,
        &input.user_id,
        turn_agent_id,
        base_prompt,
    )
    .await;

    // Stage 7f.1: Render the athlete's own open commitments. Scoped to the
    // TOOL tenant — the tenant `commitment_create` writes under and the one
    // their activity data lives in, so the block and the sweep agree on which
    // promises exist. Deliberately above the training plan (7f.2): a promise
    // the athlete made themselves outranks a plan the agent wrote for them.
    let base_prompt = inject_commitments(
        &ctx.data,
        &input.tool_tenant_id.to_string(),
        &input.user_id,
        user_timezone,
        base_prompt,
    )
    .await;

    (base_prompt, pending_followup_ids)
}
