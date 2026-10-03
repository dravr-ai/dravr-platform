// ABOUTME: Agent welcome — the opening row an agent posts, as itself, when it is bound into a thread
// ABOUTME: Deterministic (no LLM): localized title, one-line role, starter questions; stamps the introduction ledger
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The agent welcome (carnet#735).
//!
//! An athlete who picked an agent used to land in an empty thread: a header,
//! a composer, nothing else. The agent's introduction (carnet#501) only
//! opened its *first reply*, so it needed the athlete to speak first, with no
//! hint of what to ask.
//!
//! Now the agent opens the thread. When an agent is bound into a thread — a
//! conversation created with an `agent_id`, `/agent add`, the messaging
//! proposal's numeric pick, the fresh thread `/reset` forges for the same
//! agent (carnet#750) — it posts one row as itself: its title in the
//! athlete's language, its one-line role, and up to three starter questions
//! taken from its own authored Example Inputs. No model runs: the row is
//! instant, free and the same every time.
//!
//! The row counts as the agent's introduction. It is written in the same
//! transaction as the `agent_introductions` ledger row, so the agent's first
//! reply carries no introduction directive, and a second bind of the same
//! agent into the same thread finds the ledger row and posts nothing.
//!
//! The row is recognised by its [`AGENT_WELCOME_FINISH_REASON`] stamp. It
//! replays into later prompts like any assistant row — the model sees its own
//! greeting — but the starters live only in the row's actions block, and the
//! first-turn prefetch, group adoption and agent generation count around it.

use pierre_contremaitre::messaging_strings::{
    format_template, MessagingStringsRegistry, KEY_AGENT_WELCOME_GREETING,
    KEY_AGENT_WELCOME_GREETING_NO_ROLE, KEY_AGENT_WELCOME_STARTERS_TITLE,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::TranscriptSpeaker;
use pierre_core::models::onboarding::{GuidedFlow, OnboardingState};
use pierre_core::models::{
    AddMessageParams, ConversationRecord, MessageRecord, TenantId, AGENT_WELCOME_FINISH_REASON,
};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};

use crate::envelope::{ActionKind, TurnAction};
use crate::stages::command_persistence::actions_content_blocks;
use crate::stages::introduction::{collapse_whitespace, display_title, introduction_thread};
use crate::stages::persistence::fan_out_to_group_transcript;

/// The most starter questions a welcome offers. Three fit one glance and one
/// row of chips on a phone; the rest of an agent's examples stay on Discover.
pub const MAX_STARTERS: usize = 3;

/// Where an agent's welcome lands and who it speaks to.
pub struct WelcomeTarget<'a> {
    /// The conversation row the welcome is written into, as it stands with
    /// the agent bound.
    pub conversation: &'a ConversationRecord,
    /// The member of that row the welcome is addressed to.
    pub user_id: &'a str,
    /// Tenant that owns the conversation row; the ledger row is keyed under it.
    pub conversation_tenant_id: TenantId,
    /// The agent that speaks.
    pub agent_id: &'a str,
    /// Tenant the agent resolves in for this athlete.
    pub agent_tenant_id: TenantId,
    /// Locale the greeting, title, role and starters are written in.
    pub locale: &'a str,
}

/// A welcome that was written.
#[derive(Debug, Clone)]
pub struct PostedWelcome {
    /// The persisted assistant row, stamped [`AGENT_WELCOME_FINISH_REASON`],
    /// its starters in the actions block.
    pub message: MessageRecord,
    /// The same welcome as one text, starters listed, for a messaging
    /// channel — inline markdown its egress converts like a command reply.
    pub channel_text: String,
}

/// The three templates a welcome is rendered from, in one locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeTemplates {
    /// `{0}` = title, `{1}` = role.
    pub greeting: String,
    /// `{0}` = title.
    pub greeting_no_role: String,
    /// The lead-in above the starters.
    pub starters_title: String,
}

impl WelcomeTemplates {
    /// The templates for `locale`, with the registry's own fallback chain.
    #[must_use]
    pub fn resolve(strings: &MessagingStringsRegistry, locale: &str) -> Self {
        Self {
            greeting: strings.get(KEY_AGENT_WELCOME_GREETING, locale),
            greeting_no_role: strings.get(KEY_AGENT_WELCOME_GREETING_NO_ROLE, locale),
            starters_title: strings.get(KEY_AGENT_WELCOME_STARTERS_TITLE, locale),
        }
    }
}

/// A welcome's words, before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeText {
    /// The row's content: the greeting, with the role when there is one.
    pub content: String,
    /// The lead-in above [`Self::starters`]; `None` when there are none.
    pub starters_title: Option<String>,
    /// The starter questions, in authored order, at most [`MAX_STARTERS`].
    pub starters: Vec<String>,
}

impl WelcomeText {
    /// Compose the welcome of the agent titled `title`.
    ///
    /// `role` is the agent's one-line description; a blank one renders the
    /// greeting without it. `samples` are the agent's sample prompts: blanks
    /// are dropped and the first [`MAX_STARTERS`] kept.
    #[must_use]
    pub fn compose(
        templates: &WelcomeTemplates,
        title: &str,
        role: Option<&str>,
        samples: &[String],
    ) -> Self {
        let role = role.map(collapse_whitespace).filter(|r| !r.is_empty());
        let content = role.as_deref().map_or_else(
            || format_template(&templates.greeting_no_role, &[title]),
            |role| format_template(&templates.greeting, &[title, role]),
        );
        let starters: Vec<String> = samples
            .iter()
            .map(|sample| collapse_whitespace(sample))
            .filter(|sample| !sample.is_empty())
            .take(MAX_STARTERS)
            .collect();
        let starters_title = (!starters.is_empty()).then(|| templates.starters_title.clone());
        Self {
            content,
            starters_title,
            starters,
        }
    }

    /// The starters as the controls the row carries: each one sends its
    /// question as the athlete's next message.
    #[must_use]
    pub fn actions(&self) -> Vec<TurnAction> {
        self.starters
            .iter()
            .map(|question| TurnAction {
                label: question.clone(),
                kind: ActionKind::Postback,
                value: question.clone(),
            })
            .collect()
    }

    /// The welcome as one text with the starters listed, for a channel that
    /// gets it as a message rather than as buttons.
    #[must_use]
    pub fn channel_text(&self) -> String {
        let Some(title) = self.starters_title.as_deref() else {
            return self.content.clone();
        };
        let mut text = format!("{}\n\n{title}", self.content);
        for question in &self.starters {
            text.push_str("\n- ");
            text.push_str(question);
        }
        text
    }
}

/// Post `target.agent_id`'s welcome into `target.conversation`, once.
///
/// `Ok(None)`, with nothing written, when the agent has already been
/// introduced in this thread, when a guided flow owns the thread (its opener
/// is the thread's first word, and the agent's first reply introduces it as
/// before), when the agent is not one this athlete can see, when it has no
/// title to introduce itself by, or when the string catalogue holds no
/// greeting to say it with.
///
/// After the row lands the member's read marker moves past it — every bind
/// puts the athlete in front of the thread — and a group-bound row is fanned
/// out to the room transcript as the agent's.
///
/// # Errors
///
/// Returns the repository error when the agent or its translation cannot be
/// read, or the row cannot be written. Callers treat a welcome as
/// best-effort: the bind already happened and is not undone.
pub async fn post_agent_welcome(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    target: WelcomeTarget<'_>,
) -> AppResult<Option<PostedWelcome>> {
    let conversation = target.conversation;
    if guided_walk_owns(conversation) {
        return Ok(None);
    }
    let Some(text) = welcome_text(repos, strings, &target).await? else {
        return Ok(None);
    };
    let blocks = actions_content_blocks(text.starters_title.as_deref(), &text.actions())
        .map_err(|e| AppError::internal(format!("agent welcome actions not encodable: {e}")))?;
    let params = AddMessageParams {
        tenant_id: target.conversation_tenant_id,
        conversation_id: &conversation.id,
        user_id: target.user_id,
        role: "assistant",
        content: &text.content,
        token_count: None,
        finish_reason: Some(AGENT_WELCOME_FINISH_REASON),
        prompt_tokens: None,
        model: None,
        content_blocks: blocks.as_deref(),
    };
    let Some(message) = repos
        .chat
        .add_agent_welcome(&params, introduction_thread(conversation), target.agent_id)
        .await?
    else {
        return Ok(None);
    };
    after_written(repos, &target, &message).await;
    info!(
        agent_id = target.agent_id,
        conversation_id = %conversation.id,
        starters = text.starters.len(),
        "agent welcome posted"
    );
    Ok(Some(PostedWelcome {
        channel_text: text.channel_text(),
        message,
    }))
}

/// The words of `target.agent_id`'s welcome in `target.locale`, or `None`
/// when the agent is not one this athlete can see, has no title, or the
/// string catalogue holds no greeting to say it with.
async fn welcome_text(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    target: &WelcomeTarget<'_>,
) -> AppResult<Option<WelcomeText>> {
    let user = parse_uuid(target.user_id)?;
    let Some(canonical) = repos
        .agents
        .get_by_id(target.agent_id, user, target.agent_tenant_id)
        .await?
    else {
        info!(
            agent_id = target.agent_id,
            "agent not visible to the athlete; no welcome"
        );
        return Ok(None);
    };
    let mut localized = [canonical.clone()];
    repos
        .agents
        .translate_agents(&mut localized, target.locale)
        .await?;
    let [localized] = localized;
    let Some(title) = display_title(Some(&localized.title), &canonical.title) else {
        warn!(
            agent_id = target.agent_id,
            "the agent has no title to welcome by"
        );
        return Ok(None);
    };
    let text = WelcomeText::compose(
        &WelcomeTemplates::resolve(strings, target.locale),
        &title,
        localized.description.as_deref(),
        &localized.sample_prompts,
    );
    if text.content.trim().is_empty() {
        // The catalogue lacks the greeting in every locale: an instance whose
        // strings predate the welcome keys. A blank row would read as the
        // agent saying nothing at all.
        warn!(
            locale = target.locale,
            "agent welcome greeting missing from the string catalogue; no welcome"
        );
        return Ok(None);
    }
    Ok(Some(text))
}

/// Whether a guided walk (`/pillars`, a season or fortnight review) is
/// running in the thread: its opener is the thread's first word and its
/// questions the next ones, so the agent does not talk over it.
///
/// The messaging intake is not such a walk — the platform asks and parses it
/// at ingress, and the pipeline steps aside from it the same way — so an
/// agent the athlete binds while it is outstanding still welcomes them.
fn guided_walk_owns(conversation: &ConversationRecord) -> bool {
    OnboardingState::from_column(conversation.onboarding_state.as_deref())
        .is_some_and(|state| state.flow != GuidedFlow::Intake)
}

/// Move the member's read marker past the welcome and fan it out to the
/// room. Best-effort on both counts: the row is already durable.
async fn after_written(
    repos: &RepositoryRegistry,
    target: &WelcomeTarget<'_>,
    message: &MessageRecord,
) {
    let conversation = target.conversation;
    if let Err(e) = repos
        .chat
        .mark_conversation_read(
            &conversation.id,
            target.user_id,
            target.conversation_tenant_id,
            Some(&message.id),
        )
        .await
    {
        warn!(error = %e, conversation_id = %conversation.id, "read marker could not advance past the agent welcome");
    }
    if let Err(e) = fan_out_to_group_transcript(
        repos.groups.as_ref(),
        conversation,
        target.conversation_tenant_id,
        target.user_id,
        TranscriptSpeaker::Coach,
        &message.content,
        &message.id,
    )
    .await
    {
        warn!(error = %e, conversation_id = %conversation.id, "agent welcome could not reach the group transcript");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn templates() -> WelcomeTemplates {
        WelcomeTemplates {
            greeting: "Hi! Dravr's {0} here.\n\n{1}".to_owned(),
            greeting_no_role: "Hi! Dravr's {0} here.".to_owned(),
            starters_title: "To get started, you can ask me:".to_owned(),
        }
    }

    fn samples(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn greeting_carries_title_and_collapsed_role() {
        let text = WelcomeText::compose(
            &templates(),
            "Fuelling Agent",
            Some("Fuelling  specialist\nfor endurance."),
            &[],
        );
        assert_eq!(
            text.content,
            "Hi! Dravr's Fuelling Agent here.\n\nFuelling specialist for endurance."
        );
    }

    #[test]
    fn blank_role_uses_the_no_role_greeting() {
        for role in [None, Some(""), Some("  \n ")] {
            let text = WelcomeText::compose(&templates(), "Agent", role, &[]);
            assert_eq!(text.content, "Hi! Dravr's Agent here.");
        }
    }

    #[test]
    fn starters_cap_at_three_and_drop_blanks() {
        let text = WelcomeText::compose(
            &templates(),
            "Agent",
            Some("Role."),
            &samples(&["", "One?", "  ", "Two?", "Three?", "Four?"]),
        );
        assert_eq!(text.starters, samples(&["One?", "Two?", "Three?"]));
        assert_eq!(
            text.starters_title.as_deref(),
            Some("To get started, you can ask me:")
        );
        let actions = text.actions();
        assert_eq!(actions.len(), 3);
        assert!(actions
            .iter()
            .all(|a| a.kind == ActionKind::Postback && a.label == a.value));
    }

    #[test]
    fn no_starters_means_no_actions_and_no_title() {
        let text = WelcomeText::compose(&templates(), "Agent", Some("Role."), &samples(&[" "]));
        assert!(text.starters.is_empty());
        assert!(text.starters_title.is_none());
        assert!(text.actions().is_empty());
        assert_eq!(
            actions_content_blocks(None, &text.actions()).ok().flatten(),
            None
        );
        assert_eq!(text.channel_text(), text.content);
    }

    #[test]
    fn channel_text_lists_the_starters() {
        let text = WelcomeText::compose(
            &templates(),
            "Agent",
            Some("Role."),
            &samples(&["One?", "Two?"]),
        );
        assert_eq!(
            text.channel_text(),
            "Hi! Dravr's Agent here.\n\nRole.\n\nTo get started, you can ask me:\n- One?\n- Two?"
        );
    }
}
