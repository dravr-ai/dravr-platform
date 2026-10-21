// ABOUTME: The one-time onboarding agent proposal a messaging turn sends ahead of its first coached reply
// ABOUTME: Builds the inferred-profile proposal as buttons or a numbered menu, sends it, stamps the link once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;

use dravr_canot::rich_text::escape_markdown;
use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use pierre_chat_pipeline::envelope::ActionKind;
use pierre_chat_pipeline::RenderCapabilities;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_AGENT_PROPOSAL_FOOTER, KEY_AGENT_PROPOSAL_FOOTER_BUTTONS,
    KEY_AGENT_PROPOSAL_WELCOME, KEY_AGENT_PROPOSAL_WELCOME_GENERIC, KEY_DISCOVER_CARD_TITLE,
};
use pierre_core::errors::AppResult;
use pierre_core::models::messaging::{CardAction, ChannelConfig, MessageContent, OutgoingMessage};
use pierre_database::backends::MessagingRepository;
use pierre_database::repositories::OnboardingStepRecord;
use pierre_routes_agents::agents::{build_agent_proposal, ProposedAgent, SportProfileSummary};
use pierre_services::activity_sports::sport_label;
use pierre_services::analytics::hash_id;
use pierre_services::intake;
use tracing::{info, warn};

use super::surface::messaging_render_profile;
use super::viz_delivery::card_or_rich_text;
use super::PendingDispatch;

/// Auto-send the one-time onboarding agent proposal for this user, if it hasn't
/// been sent on this channel link yet.
///
/// Fires on the user's first provider-connected messaging turn: builds the
/// inferred-profile proposal (shared with the REST route via
/// [`build_agent_proposal`]), renders it to text, sends it through the channel
/// adapter, and stamps the link so it never re-sends. Entirely best-effort —
/// any failure is logged and the turn proceeds normally. When no agents are
/// eligible yet (cold start, activities not synced) it returns *without*
/// stamping, so a later turn can propose once data lands.
pub(super) async fn maybe_send_agent_proposal(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
) {
    let Some((outgoing, offered_ids)) = build_agent_proposal_message(dispatch).await else {
        return;
    };

    if let Err(e) = dispatch.adapter.send(&outgoing, channel_config).await {
        warn!(error = %e, "coach proposal: send failed; will retry next turn");
        return;
    }

    stamp_agent_proposal_sent(dispatch, &offered_ids).await;

    info!(
        hashed_user = %hash_id(&dispatch.session.user_id),
        "coach proposal auto-sent"
    );
}

/// Stamp the channel link so the proposal is never re-sent. Best-effort: a
/// failure here only risks a duplicate proposal on a later turn, never the turn.
async fn stamp_agent_proposal_sent(dispatch: &PendingDispatch, offered_ids: &[String]) {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();
    if let Err(e) = db
        .mark_agent_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
            offered_ids,
        )
        .await
    {
        warn!(error = %e, "coach proposal: sent but failed to stamp link; may re-send next turn");
    }
}

/// Whether the user's onboarding steps rule the proposal out.
///
/// A coach who does not train has no agent of their own to pick: the
/// proposal ranks agents on the reader's own activities, and their group's
/// agent is chosen when the group is created. Fails closed like the other
/// checks in [`build_agent_proposal_message`] — a read error must not offer an
/// athlete's agent to a coach.
fn withheld_for_onboarding_steps(steps: &AppResult<Vec<OnboardingStepRecord>>) -> bool {
    steps
        .as_ref()
        .map_or(true, |steps| intake::athlete_steps_waived(steps))
}

/// Decide whether to auto-send and, if so, build the outbound proposal message.
///
/// Returns `None` when the proposal was already sent (or the idempotency read
/// errored — fail closed), when the user already has an active agent (they are
/// past onboarding), when they coach others and do not train, when the build
/// fails, or when no agents are eligible yet (cold start). In the cold-start
/// case the link is intentionally left un-stamped so a later turn can propose
/// once activities sync.
async fn build_agent_proposal_message(
    dispatch: &PendingDispatch,
) -> Option<(OutgoingMessage, Vec<String>)> {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();

    let already_sent = db
        .agent_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
        )
        .await
        .unwrap_or(true); // fail closed: never risk double-sending on a read error
    if already_sent {
        return None;
    }

    // Never onboard a user who already has an active agent. The idempotency
    // flag alone is insufficient: a user can acquire an agent on the web before
    // ever receiving a messaging proposal, leaving the flag NULL — and the
    // "Welcome!" lead-in is jarring for someone mid-plan. Re-checked each turn
    // (cheap, indexed); left un-stamped so a transient read error can't
    // permanently suppress a genuinely new user's proposal.
    let has_active_agent = dispatch
        .resources
        .common
        .repos
        .agents
        .get_active_agent(dispatch.auth_result.user_id, dispatch.user_tenant_id)
        .await
        .map_or(true, |agent| agent.is_some()); // fail closed: never onboard a possibly-coached user
    if has_active_agent {
        return None;
    }

    // Left un-stamped, so a later turn re-reads the steps.
    let steps = dispatch
        .resources
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(&dispatch.auth_result.user_id.to_string())
        .await;
    if withheld_for_onboarding_steps(&steps) {
        return None;
    }

    let proposal = build_agent_proposal(
        &dispatch.resources,
        dispatch.auth_result.user_id,
        dispatch.user_tenant_id,
        &dispatch.locale,
    )
    .await
    .inspect_err(|e| warn!(error = %e, "coach proposal: build failed; skipping"))
    .ok()?;
    let (profile, agents) = (proposal.profile, proposal.agents);

    if agents.is_empty() {
        return None;
    }

    let content = agent_proposal_content(
        &messaging_render_profile(dispatch.channel_type, &dispatch.locale).render,
        &profile,
        &agents,
        &dispatch.resources.mcp.messaging_strings_registry,
        &dispatch.locale,
    );
    // Captured in the SAME order the user reads, because that ordering is what a
    // numeric reply indexes into.
    let offered_ids: Vec<String> = agents.iter().map(|c| c.agent.id.clone()).collect();
    Some((
        OutgoingMessage {
            channel_type: dispatch.channel_type,
            recipient_id: dispatch.sender_id.clone(),
            content,
            // A fresh turn id: the proposal is a proactive message, not a reply to
            // the user's inbound turn.
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: dispatch.thread_id.clone(),
        },
        offered_ids,
    ))
}

/// The onboarding agent proposal as the channel will carry it.
///
/// On a channel that draws buttons it is a card with one button per agent,
/// whose postback is the agent's number in the menu — the very reply
/// [`super::parse_choice`] reads, so a tap and a typed number pick the same
/// agent through the same path (carnet#829). canot draws the buttons as the
/// channel's own control: an inline keyboard on Telegram, reply buttons or a
/// list on `WhatsApp`. The body keeps the numbered menu with each agent's
/// reason, which a button label is too short to carry, and closes on the
/// line that says to tap. A channel without buttons gets the menu as text,
/// closing on the line that says to reply with a number.
fn agent_proposal_content(
    render: &RenderCapabilities,
    profile: &SportProfileSummary,
    agents: &[ProposedAgent],
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> MessageContent {
    let menu = render_agent_proposal_menu(profile, agents, registry, locale);
    if !render.blocks.action_buttons {
        return MessageContent::Text {
            body: menu + &registry.get(KEY_AGENT_PROPOSAL_FOOTER, locale),
        };
    }
    let body = menu + &registry.get(KEY_AGENT_PROPOSAL_FOOTER_BUTTONS, locale);
    let actions = agents
        .iter()
        .enumerate()
        .map(|(index, proposed)| CardAction {
            label: proposed.agent.title.clone(),
            action_type: ActionKind::Postback.as_str().to_owned(),
            value: (index + 1).to_string(),
        })
        .collect();
    // The menu is plain text; escaped, its titles and reasons stay literal
    // characters through the card's markdown reading. The card's title is
    // the catalogue's own heading — never empty, which Slack and Messenger
    // refuse, and short enough for the button `WhatsApp` opens a list with.
    card_or_rich_text(
        render,
        registry.get(KEY_DISCOVER_CARD_TITLE, locale),
        escape_markdown(&body),
        actions,
    )
}

/// The onboarding agent proposal's menu: a short profile-aware lead-in, then
/// a numbered list of `title — reason` lines. The closing line is the
/// caller's, since it depends on whether the channel draws buttons.
///
/// The lead-in is resolved from the messaging-strings `registry` for
/// `locale`; the numbered list is locale-neutral formatting and the
/// per-agent reasons arrive already localized from [`build_agent_proposal`].
fn render_agent_proposal_menu(
    profile: &SportProfileSummary,
    agents: &[ProposedAgent],
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> String {
    let count = agents.len().to_string();
    let mut body = profile
        .primary_sport
        .as_deref()
        .filter(|_| profile.has_profile)
        .map_or_else(
            || registry.render(KEY_AGENT_PROPOSAL_WELCOME_GENERIC, locale, &[&count]),
            |primary| {
                // The wire sport names itself in the athlete's locale when the
                // shared vocabulary knows it; an unknown spelling keeps its
                // wire text rather than inventing one.
                let sport = sport_label(registry, primary, locale);
                registry.render(KEY_AGENT_PROPOSAL_WELCOME, locale, &[&sport, &count])
            },
        );
    for (index, proposed) in agents.iter().enumerate() {
        let reason = if proposed.reason.is_empty() {
            String::new()
        } else {
            format!(" — {}", proposed.reason)
        };
        let _ = writeln!(
            body,
            "{number}. {title}{reason}",
            number = index + 1,
            title = proposed.agent.title,
        );
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::messaging_ingress::parse_choice;
    use pierre_core::errors::AppError;
    use pierre_core::models::messaging::ChannelType;
    use serde_json::json;

    const TAP_FOOTER: &str = "Tap an agent to start.";
    const NUMBER_FOOTER: &str = "Reply with a number.";

    fn proposed(title: &str, reason: &str) -> ProposedAgent {
        let agent = serde_json::from_value(json!({
            "id": format!("agent-{title}"),
            "title": title,
            "system_prompt": "You coach.",
            "category": "training",
            "tags": [],
            "token_count": 0,
            "is_favorite": false,
            "use_count": 0,
            "created_at": "2026-10-08T00:00:00Z",
            "updated_at": "2026-10-08T00:00:00Z",
            "is_system": true,
            "visibility": "tenant",
            "is_assigned": false,
        }))
        .unwrap();
        ProposedAgent {
            agent,
            match_score: 0.9,
            reason: reason.to_owned(),
        }
    }

    fn cold_start() -> SportProfileSummary {
        SportProfileSummary {
            has_profile: false,
            primary_sport: None,
            total_activities: 0,
            window_days: 30,
            sport_mix: vec![],
        }
    }

    /// The real registry, with both closing lines pinned so the test reads
    /// which one a channel got rather than the catalogue's wording.
    fn strings() -> MessagingStringsRegistry {
        let strings = MessagingStringsRegistry::new();
        strings.update(
            KEY_AGENT_PROPOSAL_FOOTER_BUTTONS,
            "en",
            TAP_FOOTER.to_owned(),
            String::new(),
        );
        strings.update(
            KEY_AGENT_PROPOSAL_FOOTER,
            "en",
            NUMBER_FOOTER.to_owned(),
            String::new(),
        );
        strings
    }

    fn agents() -> Vec<ProposedAgent> {
        vec![
            proposed("Marathon Agent", "Your long runs are building."),
            proposed("Trail Agent", "You climb a lot."),
            proposed("Fuelling Agent", "Your rides run past two hours."),
        ]
    }

    #[test]
    fn a_button_channel_gets_one_button_per_agent_posting_back_its_number() {
        let render = messaging_render_profile(ChannelType::Telegram, "en").render;
        assert!(render.blocks.action_buttons, "Telegram draws buttons");
        let agents = agents();

        let content = agent_proposal_content(&render, &cold_start(), &agents, &strings(), "en");

        let MessageContent::Card {
            title,
            body,
            actions,
        } = content
        else {
            panic!("a button channel gets a card, got {content:?}");
        };
        assert!(
            !title.trim().is_empty(),
            "Slack and Messenger refuse an empty title"
        );
        let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Marathon Agent", "Trail Agent", "Fuelling Agent"]);
        for (index, action) in actions.iter().enumerate() {
            assert_eq!(action.action_type, "postback");
            assert_eq!(
                parse_choice(&action.value),
                Some(index + 1),
                "a tap posts the number a typed reply would"
            );
        }
        assert!(body.contains("2. Trail Agent — You climb a lot."), "{body}");
        assert!(body.ends_with(TAP_FOOTER), "{body}");
        assert!(!body.contains(NUMBER_FOOTER), "{body}");
    }

    #[test]
    fn a_channel_without_buttons_keeps_the_numbered_menu_as_text() {
        let mut render = messaging_render_profile(ChannelType::Telegram, "en").render;
        render.blocks.action_buttons = false;

        let content = agent_proposal_content(&render, &cold_start(), &agents(), &strings(), "en");

        let MessageContent::Text { body } = content else {
            panic!("a channel without buttons gets text, got {content:?}");
        };
        assert!(body.contains("1. Marathon Agent — Your long runs are building."));
        assert!(body.contains("3. Fuelling Agent — Your rides run past two hours."));
        assert!(body.ends_with(NUMBER_FOOTER), "{body}");
        assert!(!body.contains(TAP_FOOTER), "{body}");
    }

    fn step(step_id: &str, status: &str) -> OnboardingStepRecord {
        OnboardingStepRecord {
            step_id: step_id.to_owned(),
            status: status.to_owned(),
            chosen_channel: None,
        }
    }

    #[test]
    fn a_coach_who_does_not_train_is_not_offered_an_agent() {
        let steps = Ok(vec![
            step(intake::STEP_PROFILE_TYPE, intake::STATUS_COMPLETE),
            step(intake::STEP_PARQ, intake::STATUS_NOT_APPLICABLE),
        ]);
        assert!(withheld_for_onboarding_steps(&steps));
    }

    #[test]
    fn an_athlete_is_still_offered_one() {
        assert!(!withheld_for_onboarding_steps(&Ok(vec![])));
        let skipped_parq = Ok(vec![step(intake::STEP_PARQ, intake::STATUS_SKIPPED)]);
        assert!(!withheld_for_onboarding_steps(&skipped_parq));
    }

    #[test]
    fn an_unreadable_record_withholds_the_proposal() {
        let steps = Err(AppError::internal("read failed"));
        assert!(withheld_for_onboarding_steps(&steps));
    }
}
