// ABOUTME: The turn subject's own athlete context — freshness, OKF bundle, playbooks, notes, followups, commitments, fuel products
// ABOUTME: One gate for prompt assembly: a coach's seat in their group skips all of it (carnet#741)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Prompt stages 7d–7f.1b: the turn subject's own athlete context.
//!
//! Everything here is read from the sender's own records. Grouped so the one
//! decision that the sender is not the subject (the coach's seat, see
//! [`super::group_subject`]) skips them together rather than stage by stage.

use std::fmt;

use pierre_services::memory_facts::SentenceRenderer;
use uuid::Uuid;

use super::super::surface_profile::SurfaceProfile;
use super::super::turn::TurnInput;
use super::commitments::inject_commitments;
use super::followups::inject_pending_followups;
use super::fuel_products::inject_fuel_products;
use super::memory::{inject_agent_notes, inject_okf_bundle, inject_playbooks};
use super::refresh::refresh_and_hint;
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
    /// Whether the fuel-products block is withheld: exactly when the plan
    /// block is — an interview owns the turn, or the turn is in a room.
    pub fuel_products_withheld: bool,
}

// The turn carries the athlete's message: only its id prints.
impl fmt::Debug for SubjectContextInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubjectContextInputs")
            .field("turn_id", &self.input.turn_id)
            .field("profile", &self.profile)
            .field("user_uuid", &self.user_uuid)
            .field("turn_agent_id", &self.turn_agent_id)
            .field("user_timezone", &self.user_timezone)
            .field("fuel_products_withheld", &self.fuel_products_withheld)
            .finish_non_exhaustive()
    }
}

/// The subject's own athlete context, as [`inject_subject_context`] built it.
pub struct SubjectContext {
    /// The system prompt with the subject's blocks appended.
    pub prompt: String,
    /// The followups it surfaced, marked delivered after the turn succeeds.
    pub pending_followup_ids: Vec<String>,
    /// The provider freshness hint, sent with the turn clock rather than in
    /// the cached prompt (see [`super::refresh`]).
    pub freshness_hint: Option<String>,
}

// The prompt holds the athlete's facts, notes and plan: only its length prints.
impl fmt::Debug for SubjectContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubjectContext")
            .field("prompt_len", &self.prompt.len())
            .field("pending_followup_ids", &self.pending_followup_ids)
            .field("freshness_hint", &self.freshness_hint)
            .finish()
    }
}

/// Append the subject's own athlete context to `base_prompt`.
pub async fn inject_subject_context(
    inputs: SubjectContextInputs<'_>,
    base_prompt: String,
) -> SubjectContext {
    let SubjectContextInputs {
        ctx,
        input,
        profile,
        user_uuid,
        turn_agent_id,
        user_timezone,
        fuel_products_withheld,
    } = inputs;

    // Stage 7d: Trigger the chat refresh and build the freshness hint.
    // How stale the athlete's provider data is has the same bearing on the
    // answer on every surface, so no surface suppresses it.
    let auth_repos = ctx.repos.auth_repos();
    let freshness_hint = refresh_and_hint(
        super::refresh::RefreshDeps {
            auth_repos: &auth_repos,
            activity_cache: ctx.repos.activity_cache.clone(),
            #[cfg(feature = "health-sync")]
            provider_connections: ctx.repos.provider_connections.clone(),
            #[cfg(feature = "health-sync")]
            groups: ctx.repos.groups.clone(),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: &ctx.sync_orchestrator,
            #[cfg(feature = "health-sync")]
            sse_manager: &ctx.sse_manager,
        },
        &input.user_id,
        input.tool_tenant_id,
    )
    .await;

    // Stage 7e: Render the per-user OKF context bundle (North Star + pillar +
    // medical facts) from the read-time Dossier into the prompt. Single
    // fact->prompt surface; user-wide (agent-agnostic) facts. Scoped to the
    // TOOL tenant — the athlete's own workspace, where every turn kind stamps
    // their facts (`turn_extraction::ExtractionTenants`), so a room turn reads
    // the goals the athlete gave in their walk or their DM.
    let base_prompt = inject_okf_bundle(
        ctx.repos.dossier.as_ref(),
        input.tool_tenant_id,
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
    // delivered after the turn succeeds. Scoped to the TOOL tenant — the
    // tenant the followup tool writes under.
    let (base_prompt, pending_followup_ids) = inject_pending_followups(
        &ctx.data,
        input.tool_tenant_id,
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

    // Stage 7f.1b: The fuel products the athlete carries, between their
    // commitments and the plan (7f.2) a day of which may name one. Scoped to
    // the TOOL tenant — the tenant `add_fuel_item` writes under — and withheld
    // where the plan block is.
    let base_prompt = inject_fuel_products(
        &ctx.repos,
        input.tool_tenant_id,
        user_uuid,
        fuel_products_withheld,
        base_prompt,
    )
    .await;

    SubjectContext {
        prompt: base_prompt,
        pending_followup_ids,
        freshness_hint,
    }
}
