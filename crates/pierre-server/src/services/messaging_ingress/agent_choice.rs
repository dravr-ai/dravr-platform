// ABOUTME: Resolves a bare numeric reply to the agent the proposal offered at that position
// ABOUTME: Implements "Reply with a number to start", binds the agent into the thread and posts its welcome

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Numeric agent selection.
//!
//! The onboarding agent proposal ends with "Reply with a number to start", and
//! nothing parsed that number. Typing `1` reached the model as ordinary
//! conversation and bound nothing — the first proactive message we ever send
//! taught the user that the bot does not do what it says.
//!
//! Resolution indexes the ids stamped alongside the proposal rather than
//! rebuilding it: the proposal is LLM-re-ranked, so a rebuild can come back in a
//! different order and bind an agent the user did not pick.

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use pierre_chat_pipeline::agent_welcome::{post_agent_welcome, PostedWelcome, WelcomeTarget};
use pierre_contremaitre::messaging_strings::KEY_AGENT_USER_UPDATED;
use pierre_core::models::agents::Agent;
use pierre_core::models::messaging::{ChannelType, IncomingMessage, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use pierre_services::messaging_broadcast::proactive_text;
use tracing::{info, warn};
use uuid::Uuid;

use super::content_body_text;
use super::otp::apply_conversation_recipient;
use super::outbound_send::{send_channel_responses, OutboundPersistSpec};
use super::session::rebind_conversation_agent;
use super::slash::welcome_content;
use super::ResolvedSession;
use crate::mcp::resources::ServerContext;

/// Parse a reply that is *only* a number.
///
/// `pub` so the integration suite can pin the strictness directly — the
/// distinction between "2" and "I run 3 times a week" is the whole safety
/// property here, and it deserves assertions of its own.
///
/// Deliberately strict: "2" selects, "2 please" and "I'll take 2" do not. A
/// loose parse would hijack ordinary conversation — someone answering "I run 3
/// times a week" must not silently rebind their agent.
#[must_use]
pub fn parse_choice(text: &str) -> Option<usize> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<usize>().ok().filter(|n| *n > 0)
}

/// Everything [`answer_agent_choice`] needs to resolve a numeric reply and
/// answer it.
///
/// A struct rather than a dozen positional parameters: the `&str`-ish fields
/// are trivially swappable at a call site, and binding the wrong agent to the
/// wrong sender is the failure this whole path exists to avoid.
pub(super) struct AgentChoiceParams<'a> {
    pub resources: &'a ServerContext,
    /// The webhook's tenant — where the channel link, and so the proposal, live.
    pub tenant_id: TenantId,
    /// The athlete's own tenant — where the proposal's agents were listed, and
    /// where their selection lives, the same tenant `/agent add` writes it in.
    pub user_tenant_id: TenantId,
    pub channel: &'a str,
    pub channel_type: ChannelType,
    pub adapter: &'a Arc<dyn MessagingChannel>,
    /// The inbound reply.
    pub message: &'a IncomingMessage,
    /// The athlete's session: the thread the pick binds the agent into.
    pub session: &'a ResolvedSession,
    /// Tenant that owns the session's conversation row.
    pub session_tenant_id: TenantId,
    pub user_id: Uuid,
    pub locale: &'a str,
    pub thread_id: Option<String>,
    /// True while the intake is waiting on an answer of its own.
    ///
    /// `parse_choice` claims ANY bare digit, so a "3" typed at a PAR-Q question
    /// would bind an agent the athlete never picked. The intake used to shadow
    /// this band by handling such a turn outright; now that it yields the turn,
    /// the exclusion has to be stated here.
    pub intake_awaiting: bool,
}

/// Answer a numeric reply to the agent proposal, if that is what this is.
///
/// Returns `false` when the message is not a bare number, when no proposal is
/// outstanding (none was sent, or the athlete already picked from it), or
/// when the number is out of range — in every case the turn
/// continues to the model untouched, which is exactly the previous behaviour.
///
/// A pick is the messaging equivalent of the app's « Démarrer »: the agent is
/// selected, bound into the session's thread at once rather than on the next
/// turn, and opens it with its welcome (carnet#735), delivered after the
/// confirmation.
pub(super) async fn answer_agent_choice(params: AgentChoiceParams<'_>) -> bool {
    let Some((agent_id, agent)) = select_chosen_agent(&params).await else {
        return false;
    };
    let body = params.resources.mcp.messaging_strings_registry.render(
        KEY_AGENT_USER_UPDATED,
        params.locale,
        &[&agent.title],
    );
    let confirmation = proactive_text(params.channel_type, params.message.sender_id.clone(), body);
    let mut answers = vec![(confirmation, None)];

    let session = params.session;
    rebind_conversation_agent(
        params.resources,
        params.session_tenant_id,
        &session.user_id,
        &session.conversation,
    )
    .await;
    if let Some(welcome) = welcome_chosen_agent(&params, &agent_id).await {
        let message = OutgoingMessage {
            content: welcome_content(&welcome.channel_text),
            ..proactive_text(
                params.channel_type,
                params.message.sender_id.clone(),
                String::new(),
            )
        };
        let ledger = OutboundPersistSpec {
            db: Arc::clone(&params.resources.common.repos.messaging),
            session_tenant_id: params.session_tenant_id,
            session_id: session.session_id.clone(),
            chat_message_id: Some(welcome.message.id),
        };
        answers.push((message, Some(ledger)));
    }
    deliver(&params, answers).await;
    true
}

/// Resolve the pick to an offered agent the athlete can see and select it.
/// `None` for anything that is not such a pick, or when the selection could
/// not be written.
async fn select_chosen_agent(params: &AgentChoiceParams<'_>) -> Option<(String, Agent)> {
    if params.intake_awaiting {
        return None;
    }
    let text = content_body_text(&params.message.content)?;
    let choice = parse_choice(&text)?;
    let resources = params.resources;
    let sender_id = params.message.sender_id.as_str();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let offered = db
        .proposed_agent_ids(params.tenant_id, params.channel, sender_id)
        .await
        .unwrap_or_default();
    if offered.is_empty() {
        return None;
    }

    // Out of range is not a selection. Falling through to the model beats
    // guessing, and beats an error for someone who just happened to type "9".
    let agent_id = offered.get(choice - 1)?;

    let agent = resources
        .common
        .repos
        .agents
        .get_by_id(agent_id, params.user_id, params.user_tenant_id)
        .await
        .ok()
        .flatten()?;

    if !apply_pick(params, agent_id).await {
        return None;
    }

    info!(
        agent_id = %agent_id,
        choice,
        "coach bound from a numeric reply to the proposal"
    );
    Some((agent_id.clone(), agent))
}

/// Select `agent_id` for the athlete and spend the proposal it came from.
/// `false` when the selection could not be written.
async fn apply_pick(params: &AgentChoiceParams<'_>, agent_id: &str) -> bool {
    let repos = &params.resources.common.repos;
    // The one selection pointer — same write `/agent add` performs, so a
    // user ends up with an agent exactly one way regardless of how they said so.
    if let Err(e) = repos
        .tenants
        .set_selected_agent(params.user_tenant_id, params.user_id, Some(agent_id))
        .await
    {
        warn!(error = %e, agent_id = %agent_id, "failed to bind coach from numeric reply");
        return false;
    }

    // The offer is spent: the athlete answered it. A bare number from here on
    // is conversation — the agent's own "how many gels did you take?" gets
    // its "2". Best-effort: the pick stands even when the clear fails, and
    // the next digit would at worst re-pick from the same offer.
    if let Err(e) = repos
        .messaging
        .clear_proposed_agent_ids(
            params.tenant_id,
            params.channel,
            params.message.sender_id.as_str(),
        )
        .await
    {
        warn!(error = %e, agent_id = %agent_id, "the answered agent proposal could not be cleared");
    }
    true
}

/// Post the chosen agent's welcome into the session's thread, once the
/// rebind has pointed the thread at it. Best-effort: the selection stands
/// whatever happens here.
async fn welcome_chosen_agent(
    params: &AgentChoiceParams<'_>,
    agent_id: &str,
) -> Option<PostedWelcome> {
    let repos = &params.resources.common.repos;
    let session = params.session;
    let conversation = repos
        .chat
        .get_conversation(
            &session.conversation,
            &session.user_id,
            params.session_tenant_id,
        )
        .await
        .ok()
        .flatten()?;
    if conversation.agent_id.as_deref() != Some(agent_id) {
        return None;
    }
    let target = WelcomeTarget {
        conversation: &conversation,
        user_id: &session.user_id,
        conversation_tenant_id: params.session_tenant_id,
        agent_id,
        agent_tenant_id: params.user_tenant_id,
        locale: params.locale,
    };
    match post_agent_welcome(
        repos,
        &params.resources.mcp.messaging_strings_registry,
        target,
    )
    .await
    {
        Ok(welcome) => welcome,
        Err(e) => {
            warn!(error = %e, agent_id, "agent welcome not posted after the numeric pick");
            None
        }
    }
}

/// Send the pick's answers in order from one delivery, each addressed and
/// threaded like the reply it is: sent one by one they would race, and the
/// welcome could arrive before the confirmation it follows.
async fn deliver(
    params: &AgentChoiceParams<'_>,
    mut answers: Vec<(OutgoingMessage, Option<OutboundPersistSpec>)>,
) {
    for (message, _) in &mut answers {
        message.thread_id.clone_from(&params.thread_id);
        apply_conversation_recipient(message, params.message.conversation_id.as_deref());
    }
    send_channel_responses(
        params.resources,
        params.tenant_id,
        params.channel,
        params.adapter,
        answers,
    )
    .await;
}
