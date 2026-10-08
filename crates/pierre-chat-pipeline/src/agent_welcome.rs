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
//! athlete's language, its one-line role, and up to three starters. No model
//! runs: the row is instant and free.
//!
//! The starters are catalogue use cases ranked from the athlete's state
//! (carnet#828) — "Connect my watch" for an athlete with nothing connected,
//! "My last workout" for one with rides — then the agent's own Example Inputs
//! in the slots left, offered only to an athlete who meets the agent's
//! prerequisites. A room keeps the examples alone (D9): ranking reads one
//! athlete's state, and the whole room would read it back.
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

use chrono::Utc;
use pierre_contremaitre::messaging_strings::{
    format_template, MessagingStringsRegistry, KEY_AGENT_WELCOME_GREETING,
    KEY_AGENT_WELCOME_GREETING_NO_ROLE, KEY_AGENT_WELCOME_STARTERS_TITLE,
};
use pierre_contremaitre::use_case_catalogue::{UseCase, UseCaseCatalogue, UseCaseRun};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::TranscriptSpeaker;
use pierre_core::models::onboarding::{GuidedFlow, OnboardingState};
use pierre_core::models::{
    AddMessageParams, Agent, ConversationRecord, MessageRecord, TenantId,
    AGENT_WELCOME_FINISH_REASON,
};
use pierre_core::transport::TransportPolicy;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::RepositoryRegistry;
use pierre_services::agents::meets_prerequisites;
use pierre_services::athlete_state::AthleteState;
use tracing::{info, warn};
use uuid::Uuid;

use crate::envelope::{ActionKind, TurnAction};
use crate::stages::command_persistence::actions_content_blocks;
use crate::stages::introduction::{collapse_whitespace, display_title, introduction_thread};
use crate::stages::persistence::fan_out_to_group_transcript;
use crate::suggestions::{
    emit_shown, rank, rotation_for, Postback, SuggestionEvent, SURFACE_AGENT_WELCOME,
};
use crate::surface_profile::SurfaceId;

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
    /// The chat surface the bind happened on, reported with each starter.
    pub surface: SurfaceId,
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

/// One starter a welcome offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Starter {
    /// What its button shows.
    pub label: String,
    /// How a channel that gets the welcome as text lists it: a command
    /// starter as `/command — label`, a prompt starter as its prompt, an
    /// example as its question.
    pub line: String,
    /// What a tap sends.
    pub(crate) postback: Postback,
}

/// A welcome's words, before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeText {
    /// The row's content: the greeting, with the role when there is one.
    pub content: String,
    /// The lead-in above [`Self::starters`]; `None` when there are none.
    pub starters_title: Option<String>,
    /// The starters, slot by slot, at most [`MAX_STARTERS`].
    pub starters: Vec<Starter>,
}

impl WelcomeText {
    /// Compose the welcome of the agent titled `title`, offering `starters`.
    ///
    /// `role` is the agent's one-line description; a blank one renders the
    /// greeting without it.
    #[must_use]
    pub fn compose(
        templates: &WelcomeTemplates,
        title: &str,
        role: Option<&str>,
        starters: Vec<Starter>,
    ) -> Self {
        let role = role.map(collapse_whitespace).filter(|r| !r.is_empty());
        let content = role.as_deref().map_or_else(
            || format_template(&templates.greeting_no_role, &[title]),
            |role| format_template(&templates.greeting, &[title, role]),
        );
        let starters_title = (!starters.is_empty()).then(|| templates.starters_title.clone());
        Self {
            content,
            starters_title,
            starters,
        }
    }

    /// The starters' postbacks, slot by slot.
    #[must_use]
    pub(crate) fn postbacks(&self) -> Vec<Postback> {
        self.starters.iter().map(|s| s.postback.clone()).collect()
    }

    /// The starters as the controls the row carries. Each shows its label and
    /// sends its postback, which the turn resolves into the command or the
    /// words it stands for (carnet#828) — so a tap reaches the transcript as
    /// those, and is counted as a tap rather than as typing.
    #[must_use]
    pub fn actions(&self) -> Vec<TurnAction> {
        self.starters
            .iter()
            .map(|starter| TurnAction {
                label: starter.label.clone(),
                kind: ActionKind::Postback,
                value: starter.postback.encode(),
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
        for starter in &self.starters {
            text.push_str("\n- ");
            text.push_str(&starter.line);
        }
        text
    }
}

/// The agent's Example Inputs as starters for the slots from `first_slot`,
/// at most `room` of them, in authored order. A postback's index counts in
/// [`starter_samples`], so it points at the same words when it is tapped.
#[must_use]
pub(crate) fn example_starters(samples: &[String], first_slot: usize, room: usize) -> Vec<Starter> {
    starter_samples(samples)
        .into_iter()
        .take(room)
        .enumerate()
        .map(|(index, question)| Starter {
            label: question.clone(),
            line: question,
            postback: Postback::Example {
                position: first_slot + index,
                index,
            },
        })
        .collect()
}

/// The catalogue starter `entry` in `locale`, offered in slot `position`, or
/// `None` when the strings hold no words for it.
fn use_case_starter(
    entry: &UseCase,
    strings: &MessagingStringsRegistry,
    locale: &str,
    position: usize,
) -> Option<Starter> {
    let label = strings.get(&entry.label_key(), locale);
    if label.trim().is_empty() {
        return None;
    }
    let line = match &entry.run {
        UseCaseRun::Command(command) => format!("{command} — {label}"),
        UseCaseRun::Prompt => strings.get(&entry.prompt_key(), locale),
    };
    if line.trim().is_empty() {
        return None;
    }
    Some(Starter {
        label,
        line,
        postback: Postback::UseCase {
            position,
            id: entry.id.clone(),
        },
    })
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
    let user = parse_uuid(target.user_id)?;
    let Some((text, transport_policy)) = welcome_text(repos, strings, &target, user).await? else {
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
        // Starters chosen from the athlete's state carry what it was read from.
        transport_policy,
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
    // `suggestion.shown` counts a starter the athlete could tap, and so does
    // the exposure the ranker retires starters by. A messaging channel
    // receives the welcome as text (`channel_text`), where a starter typed
    // back is the athlete's own words, so only the app's surfaces count.
    if matches!(target.surface, SurfaceId::Web | SurfaceId::Mobile) {
        record_shown(repos, &target, user, &text).await;
    }
    Ok(Some(PostedWelcome {
        channel_text: text.channel_text(),
        message,
    }))
}

/// Report each starter shown, and count the ranked ones toward retiring
/// them. Best-effort: an uncounted showing only lets a starter be offered
/// once more.
async fn record_shown(
    repos: &RepositoryRegistry,
    target: &WelcomeTarget<'_>,
    user: Uuid,
    text: &WelcomeText,
) {
    let shown = SuggestionEvent {
        user_id: target.user_id,
        tenant_id: target.conversation_tenant_id,
        surface: SURFACE_AGENT_WELCOME,
        channel: target.surface.as_str(),
    };
    let postbacks = text.postbacks();
    for postback in &postbacks {
        emit_shown(&shown, postback);
    }
    let ranked: Vec<&str> = postbacks
        .iter()
        .filter_map(|postback| match postback {
            Postback::UseCase { id, .. } => Some(id.as_str()),
            Postback::Example { .. } => None,
        })
        .collect();
    if ranked.is_empty() {
        return;
    }
    if let Err(e) = repos
        .use_case_exposures
        .record_use_cases_shown(target.agent_tenant_id, user, &ranked, Utc::now())
        .await
    {
        warn!(error = %e, conversation_id = %target.conversation.id, "starters shown could not be counted");
    }
}

/// The words of `target.agent_id`'s welcome in `target.locale`, and the
/// policy the row must be stamped with; `None` when the agent is not one
/// this athlete can see, has no title, or the string catalogue holds no
/// greeting to say it with.
async fn welcome_text(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    target: &WelcomeTarget<'_>,
    user: Uuid,
) -> AppResult<Option<(WelcomeText, TransportPolicy)>> {
    let Some((canonical, localized)) = localized_agent(
        repos,
        target.agent_id,
        user,
        target.agent_tenant_id,
        target.locale,
    )
    .await?
    else {
        info!(
            agent_id = target.agent_id,
            "agent not visible to the athlete; no welcome"
        );
        return Ok(None);
    };
    let Some(title) = display_title(Some(&localized.title), &canonical.title) else {
        warn!(
            agent_id = target.agent_id,
            "the agent has no title to welcome by"
        );
        return Ok(None);
    };
    let (starters, transport_policy) =
        choose_starters(repos, strings, target, &canonical, &localized, user).await;
    let text = WelcomeText::compose(
        &WelcomeTemplates::resolve(strings, target.locale),
        &title,
        localized.description.as_deref(),
        starters,
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
    Ok(Some((text, transport_policy)))
}

/// The starters `target`'s welcome offers, and the policy the row must be
/// stamped with.
///
/// Ranked catalogue starters first, then the agent's Example Inputs in the
/// slots left — only for an athlete who meets the agent's prerequisites, so
/// "analyse my last ride" never reaches one with nothing connected. A room
/// keeps the examples alone (D9). When the athlete's state cannot be read the
/// welcome still goes out, with the examples.
async fn choose_starters(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    target: &WelcomeTarget<'_>,
    agent: &Agent,
    localized: &Agent,
    user: Uuid,
) -> (Vec<Starter>, TransportPolicy) {
    let examples_only = || {
        (
            example_starters(&localized.sample_prompts, 0, MAX_STARTERS),
            TransportPolicy::AnyTransport,
        )
    };
    if target.conversation.group_id.is_some() {
        return examples_only();
    }
    let (state, transport_policy) = match AthleteState::read(
        repos,
        target.agent_tenant_id,
        user,
        Utc::now(),
    )
    .await
    {
        Ok(read) => read,
        Err(e) => {
            warn!(error = %e, conversation_id = %target.conversation.id, "athlete state unreadable; the welcome offers the agent's examples");
            return examples_only();
        }
    };
    let exposures = repos
        .use_case_exposures
        .use_case_exposures(target.agent_tenant_id, user)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, conversation_id = %target.conversation.id, "starter exposures unreadable; ranking as if none were shown");
            Vec::new()
        });
    let ranked = rank(
        UseCaseCatalogue::pinned(),
        &state,
        agent.category,
        &exposures,
        rotation_for(&target.conversation.id),
    );
    let mut starters = Vec::with_capacity(MAX_STARTERS);
    for entry in ranked {
        if let Some(starter) = use_case_starter(entry, strings, target.locale, starters.len()) {
            starters.push(starter);
        }
    }
    if meets_prerequisites(&agent.prerequisites, state.has_provider) {
        let first_slot = starters.len();
        starters.extend(example_starters(
            &localized.sample_prompts,
            first_slot,
            MAX_STARTERS.saturating_sub(first_slot),
        ));
    }
    (starters, transport_policy)
}

/// `agent_id` as `user` sees it — canonical, and translated into `locale` —
/// or `None` when it is not an agent they can see.
///
/// One read for the welcome and for resolving a tapped starter, so the
/// examples a postback indexes are the ones the welcome offered.
pub(crate) async fn localized_agent(
    repos: &RepositoryRegistry,
    agent_id: &str,
    user: Uuid,
    tenant_id: TenantId,
    locale: &str,
) -> AppResult<Option<(Agent, Agent)>> {
    let Some(canonical) = repos.agents.get_by_id(agent_id, user, tenant_id).await? else {
        return Ok(None);
    };
    let mut localized = [canonical.clone()];
    repos
        .agents
        .translate_agents(&mut localized, locale)
        .await?;
    let [localized] = localized;
    Ok(Some((canonical, localized)))
}

/// An agent's Example Inputs as starters: whitespace collapsed, blanks
/// dropped, in authored order. A starter postback's index counts in this list.
#[must_use]
pub(crate) fn starter_samples(samples: &[String]) -> Vec<String> {
    samples
        .iter()
        .map(|sample| collapse_whitespace(sample))
        .filter(|sample| !sample.is_empty())
        .collect()
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
        message,
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

    fn examples(items: &[&str]) -> Vec<Starter> {
        example_starters(&samples(items), 0, MAX_STARTERS)
    }

    fn labels(starters: &[Starter]) -> Vec<&str> {
        starters.iter().map(|s| s.label.as_str()).collect()
    }

    const CATALOGUE: &str = "
- {id: about_me, run: command, command: /pillars, requires: [], stage: any, domains: [any], repeat: once}
- {id: last_workout, run: prompt, command: null, requires: [], stage: any, domains: [training], repeat: recurring}
- {id: not_in_the_strings, run: prompt, command: null, requires: [], stage: any, domains: [training], repeat: once}
";

    #[test]
    fn greeting_carries_title_and_collapsed_role() {
        let text = WelcomeText::compose(
            &templates(),
            "Fuelling Agent",
            Some("Fuelling  specialist\nfor endurance."),
            Vec::new(),
        );
        assert_eq!(
            text.content,
            "Hi! Dravr's Fuelling Agent here.\n\nFuelling specialist for endurance."
        );
    }

    #[test]
    fn blank_role_uses_the_no_role_greeting() {
        for role in [None, Some(""), Some("  \n ")] {
            let text = WelcomeText::compose(&templates(), "Agent", role, Vec::new());
            assert_eq!(text.content, "Hi! Dravr's Agent here.");
        }
    }

    #[test]
    fn examples_take_the_room_they_are_given_and_drop_blanks() {
        let authored = samples(&["", "One?", "  ", "Two?", "Three?", "Four?"]);
        assert_eq!(
            labels(&example_starters(&authored, 0, MAX_STARTERS)),
            ["One?", "Two?", "Three?"]
        );
        let after_two_ranked = example_starters(&authored, 2, 1);
        assert_eq!(labels(&after_two_ranked), ["One?"]);
        assert_eq!(after_two_ranked[0].postback.encode(), "ex:2:0");
        assert!(example_starters(&authored, 3, 0).is_empty());
    }

    #[test]
    fn a_starter_shows_its_label_and_sends_its_postback() {
        let text = WelcomeText::compose(
            &templates(),
            "Agent",
            Some("Role."),
            examples(&["", "One?", "Two?"]),
        );
        assert_eq!(
            text.starters_title.as_deref(),
            Some("To get started, you can ask me:")
        );
        let actions = text.actions();
        let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).collect();
        let values: Vec<&str> = actions.iter().map(|a| a.value.as_str()).collect();
        assert_eq!(labels, ["One?", "Two?"]);
        assert_eq!(values, ["ex:0:0", "ex:1:1"]);
        assert!(actions.iter().all(|a| a.kind == ActionKind::Postback));
    }

    #[test]
    fn a_postback_index_counts_in_the_normalized_examples() {
        let authored = samples(&["", "  One?  ", " ", "Two\nlines?"]);
        assert_eq!(starter_samples(&authored), samples(&["One?", "Two lines?"]));
        let offered = example_starters(&authored, 0, MAX_STARTERS);
        assert_eq!(labels(&offered), ["One?", "Two lines?"]);
        assert_eq!(offered[1].postback.encode(), "ex:1:1");
    }

    #[test]
    fn a_ranked_starter_is_labelled_from_the_strings_and_lists_what_it_runs() {
        let catalogue = UseCaseCatalogue::parse(CATALOGUE).expect("the fixture parses");
        let strings = MessagingStringsRegistry::new();
        let starter =
            |id: &str, slot| use_case_starter(catalogue.get(id).expect(id), &strings, "en", slot);

        let about_me = starter("about_me", 0).expect("about_me has words");
        assert_eq!(
            about_me.label,
            strings.get("use_cases.about_me.label", "en")
        );
        assert_eq!(about_me.line, format!("/pillars — {}", about_me.label));
        assert_eq!(about_me.postback.encode(), "uc:0:about_me");

        let last = starter("last_workout", 1).expect("last_workout has words");
        assert_eq!(
            last.line,
            strings.get("use_cases.last_workout.prompt", "en")
        );
        assert_eq!(last.postback.encode(), "uc:1:last_workout");

        assert_eq!(starter("not_in_the_strings", 2), None);
    }

    #[test]
    fn no_starters_means_no_actions_and_no_title() {
        let text = WelcomeText::compose(&templates(), "Agent", Some("Role."), examples(&[" "]));
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
            examples(&["One?", "Two?"]),
        );
        assert_eq!(
            text.channel_text(),
            "Hi! Dravr's Agent here.\n\nRole.\n\nTo get started, you can ask me:\n- One?\n- Two?"
        );
        let opaque = ["uc:", "ex:"];
        assert!(!opaque
            .iter()
            .any(|prefix| text.channel_text().contains(prefix)));
    }
}
