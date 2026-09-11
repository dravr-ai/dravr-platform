// ABOUTME: The one place an agent is bound to a conversation and agent.selected is emitted
// ABOUTME: Shared by the REST usage endpoint, web chat, /agent add, and messaging ingress

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent-selection recording.
//!
//! Four surfaces bind an agent to a conversation — `POST
//! /api/agents/{id}/usage` (what the web Coaches UI and onboarding
//! proposal call when the athlete picks one), web chat conversation
//! creation, the `/agent add` slash command, and messaging session
//! creation. All four bump the same `agent_assignments.use_count`, so all
//! four are the same product event.
//!
//! Only the REST route used to emit `agent.selected`, which made the metric
//! read as "nobody picks coaches" while every chat user picked one — the
//! event belongs to the domain operation, not to whichever transport
//! happened to trigger it.
//!
//! The surfaces are not equally meaningful, though, which is why every
//! emission carries a [`AgentSelectionSource`]: a REST bump and a
//! `/agent add` are an athlete actively choosing, while a conversation
//! create re-reports the choice they already made. Counting them together
//! answers "how much is coaching used", counting
//! [`AgentSelectionSource::Rest`] and [`AgentSelectionSource::SlashCommand`]
//! alone answers "how many people choose a coach" — the question that
//! motivated moving this off the REST route in the first place.

use std::fmt;

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::AgentsRepository;
use tracing::{info, warn};
use uuid::Uuid;

/// Which surface bound the agent, carried on `agent.selected` as `source`.
///
/// An additive field — the catalogue's `required_fields` for the event are
/// `user_id`, `tenant_id` and `agent_slug`, so this narrows the metric
/// without changing its contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSelectionSource {
    /// `POST /api/agents/{id}/usage` — the web Coaches UI and the
    /// onboarding proposal. An explicit pick.
    Rest,
    /// The `/agent add` (or `/agent assign`) slash command on any chat
    /// surface. An explicit pick.
    SlashCommand,
    /// Web chat conversation creation, binding the already-selected agent.
    ChatConversation,
    /// Messaging session creation, binding the already-selected agent. Fires
    /// again whenever a session rolls over, so it counts conversations rather
    /// than choices.
    MessagingSession,
}

impl AgentSelectionSource {
    /// Stable wire value for the event field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::SlashCommand => "slash_command",
            Self::ChatConversation => "chat_conversation",
            Self::MessagingSession => "messaging_session",
        }
    }
}

impl fmt::Display for AgentSelectionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Record that `agent_id` was selected for a conversation and emit the
/// catalogued `agent.selected` event.
///
/// Returns whether the usage bump landed. `Ok(false)` means the agent is not
/// visible to this tenant (a caller passing an id they cannot see); no event
/// is emitted in that case, because nothing was selected.
///
/// `user_id` and `tenant_id` ride on the event inline rather than being left
/// to the enclosing span: the messaging ingress span carries neither, so a
/// span-only event would be dropped by the `PostHog` sink, which keys
/// `distinct_id` off `user_id`.
///
/// `source` records which surface bound the agent, so an explicit pick can be
/// told apart from a conversation re-reporting one.
///
/// # Errors
///
/// Returns the database error if the usage bump fails.
pub async fn record_agent_selection(
    agents: &dyn AgentsRepository,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    source: AgentSelectionSource,
) -> AppResult<bool> {
    let recorded = agents.record_usage(agent_id, user_id, tenant_id).await?;
    if !recorded {
        warn!(
            agent_id,
            %tenant_id,
            source = source.as_str(),
            "skipping coach usage bump — coach not visible to caller's tenant"
        );
        return Ok(false);
    }

    // `agent_slug` is the catalogue's field name for the agent identifier the
    // routing rule keys on — the same value every surface passes as
    // `agent_id`.
    info!(
        target: "notify",
        event = "agent.selected",
        user_id = %user_id,
        tenant_id = %tenant_id,
        agent_slug = %agent_id,
        source = source.as_str(),
        "user selected coach"
    );
    Ok(true)
}
