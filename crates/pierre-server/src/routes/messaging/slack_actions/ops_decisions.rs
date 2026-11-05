// ABOUTME: The decisions an operator takes from a notify card: their buttons, their action ids and the room each is honoured in
// ABOUTME: Approve or reject a sign-up, grant or decline coach access, raise a capped athlete, answer feedback — never from another room
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use dravr_tronc::notify::NotifyActions;
use pierre_contremaitre::notify_routing::ContremaitreRoutingProvider;
use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use super::feedback_reply_tap::{FeedbackKind, FeedbackRef, FeedbackTap};
use super::quota_unlock_tap::raise_can_lift;
use super::SlackAction;
// The notify event the Reply and Show full text buttons go out under; its
// routing rule names the room they are honoured in, and the room a reply's
// submission is re-checked against.
use crate::services::feedback_events::FEEDBACK_RECORDED_EVENT;

/// What an ops button decides, read back from its `action_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpsAction {
    /// Approve or reject the account waiting under `user.pending_approval`.
    SignUp(ActionType, Uuid),
    /// Grant or decline the request under `coach_access.requested`.
    CoachAccess(CoachAccessDecision, Uuid),
    /// Raise the athlete a `quota.limit_reached` notice names to Enterprise.
    RaiseToEnterprise(Uuid),
    /// Reply to, or read in full, the feedback a `feedback.recorded` card
    /// names.
    Feedback(FeedbackTap, FeedbackRef),
}

impl OpsAction {
    /// The notify event this action's buttons go out under; its routing rule
    /// names the room the tap is honoured in.
    pub(super) const fn event(self) -> &'static str {
        match self {
            Self::SignUp(..) => PENDING_APPROVAL_EVENT,
            Self::CoachAccess(..) => COACH_ACCESS_REQUESTED_EVENT,
            Self::RaiseToEnterprise(_) => QUOTA_LIMIT_REACHED_EVENT,
            Self::Feedback(..) => FEEDBACK_RECORDED_EVENT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionType {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CoachAccessDecision {
    Grant,
    Decline,
}

/// `action_id` prefix of the Approve button; the account's id follows it.
const APPROVE_USER_PREFIX: &str = "approve_user:";

/// `action_id` prefix of the Reject button; the account's id follows it.
const REJECT_USER_PREFIX: &str = "reject_user:";

/// `action_id` prefix of the Grant button; the coach-access request's id
/// follows it.
const GRANT_COACH_ACCESS_PREFIX: &str = "grant_coach_access:";

/// `action_id` prefix of the Decline button; the coach-access request's id
/// follows it.
const DECLINE_COACH_ACCESS_PREFIX: &str = "decline_coach_access:";

/// `action_id` prefix of the Raise to Enterprise button; the capped athlete's
/// id follows it.
const RAISE_TIER_ENTERPRISE_PREFIX: &str = "raise_tier_enterprise:";

/// `action_id` prefix of the Reply button; the feedback's
/// [`FeedbackRef::encode`] follows it.
const FEEDBACK_REPLY_PREFIX: &str = "feedback_reply:";

/// `action_id` prefix of the Show full text button; the feedback's
/// [`FeedbackRef::encode`] follows it.
const FEEDBACK_TEXT_PREFIX: &str = "feedback_text:";

/// Read an ops button's `action_id` back: one of the prefixes
/// [`OpsDecisionActions`] posts, followed by the id the tap acts on.
pub(super) fn parse_action_id(action_id: &str) -> AppResult<OpsAction> {
    let id = |raw: &str| {
        Uuid::parse_str(raw)
            .map_err(|e| AppError::invalid_input(format!("Invalid ID in action: {e}")))
    };
    if let Some(raw) = action_id.strip_prefix(APPROVE_USER_PREFIX) {
        Ok(OpsAction::SignUp(ActionType::Approve, id(raw)?))
    } else if let Some(raw) = action_id.strip_prefix(REJECT_USER_PREFIX) {
        Ok(OpsAction::SignUp(ActionType::Reject, id(raw)?))
    } else if let Some(raw) = action_id.strip_prefix(GRANT_COACH_ACCESS_PREFIX) {
        Ok(OpsAction::CoachAccess(CoachAccessDecision::Grant, id(raw)?))
    } else if let Some(raw) = action_id.strip_prefix(DECLINE_COACH_ACCESS_PREFIX) {
        Ok(OpsAction::CoachAccess(
            CoachAccessDecision::Decline,
            id(raw)?,
        ))
    } else if let Some(raw) = action_id.strip_prefix(RAISE_TIER_ENTERPRISE_PREFIX) {
        Ok(OpsAction::RaiseToEnterprise(id(raw)?))
    } else if let Some(raw) = action_id.strip_prefix(FEEDBACK_REPLY_PREFIX) {
        Ok(OpsAction::Feedback(
            FeedbackTap::Reply,
            FeedbackRef::parse(raw)?,
        ))
    } else if let Some(raw) = action_id.strip_prefix(FEEDBACK_TEXT_PREFIX) {
        Ok(OpsAction::Feedback(
            FeedbackTap::FullText,
            FeedbackRef::parse(raw)?,
        ))
    } else {
        Err(AppError::invalid_input(format!(
            "Unknown action: {action_id}"
        )))
    }
}

/// The notify event the Approve and Reject buttons go out under; its routing
/// rule names the room they are honoured in.
const PENDING_APPROVAL_EVENT: &str = "user.pending_approval";

/// The notify event the Grant and Decline buttons go out under; its routing
/// rule names the room they are honoured in.
const COACH_ACCESS_REQUESTED_EVENT: &str = "coach_access.requested";

/// The notify event the Raise to Enterprise button goes out under; its routing
/// rule names the room it is honoured in.
const QUOTA_LIMIT_REACHED_EVENT: &str = "quota.limit_reached";

/// Posts the buttons an operator decides from in Slack.
///
/// Approve and Reject under `user.pending_approval`, Grant and Decline under
/// `coach_access.requested`, Raise to Enterprise under `quota.limit_reached`,
/// Reply and Show full text under `feedback.recorded`.
///
/// Registered once at startup via [`pierre_logging::set_notify_actions`],
/// which takes a single provider, so every ops event's buttons are posted
/// here. The `NotifyLayer` calls it on the tracing thread, so it reads only
/// the event's fields. Each button's `action_id` is a prefix
/// [`parse_action_id`] reads followed by the id the tap acts on — the waiting
/// account's `user_id`, the coach-access `request_id`, the capped athlete's
/// `user_id`; an event without it gets no buttons, since a tap could not say
/// what it acts on. A capped athlete a raise could not unlock gets none
/// either ([`raise_can_lift`]). A feedback card's buttons name the feedback
/// by kind, tenant, athlete and row ([`feedback_buttons`]).
#[derive(Debug)]
pub struct OpsDecisionActions;

impl NotifyActions for OpsDecisionActions {
    fn actions_for(&self, event: &str, fields: &HashMap<String, String>) -> Option<Vec<Value>> {
        if event == FEEDBACK_RECORDED_EVENT {
            return feedback_buttons(fields);
        }
        let (id_field, buttons): (&str, &[DecisionButton]) = match event {
            PENDING_APPROVAL_EVENT => (
                "user_id",
                &[
                    (APPROVE_USER_PREFIX, "Approve", "primary"),
                    (REJECT_USER_PREFIX, "Reject", "danger"),
                ],
            ),
            COACH_ACCESS_REQUESTED_EVENT => (
                "request_id",
                &[
                    (GRANT_COACH_ACCESS_PREFIX, "Grant", "primary"),
                    (DECLINE_COACH_ACCESS_PREFIX, "Decline", "danger"),
                ],
            ),
            QUOTA_LIMIT_REACHED_EVENT => (
                "user_id",
                &[(
                    RAISE_TIER_ENTERPRISE_PREFIX,
                    "Raise to Enterprise",
                    "primary",
                )],
            ),
            _ => return None,
        };
        let id = fields.get(id_field)?;
        if event == QUOTA_LIMIT_REACHED_EVENT && !raise_can_lift(fields) {
            return None;
        }
        Some(
            buttons
                .iter()
                .map(|&button| decision_button(button, id))
                .collect(),
        )
    }
}

/// An ops button as `(action_id prefix, label, Slack style)`.
type DecisionButton = (&'static str, &'static str, &'static str);

/// One ops button: `label`, styled `style`, whose `action_id` is `prefix`
/// followed by `id`.
fn decision_button((prefix, label, style): DecisionButton, id: &str) -> Value {
    json!({
        "type": "button",
        "action_id": format!("{prefix}{id}"),
        "text": { "type": "plain_text", "text": label },
        "style": style,
    })
}

/// The Reply and Show full text buttons of a `feedback.recorded` card, or
/// none when the event does not name its feedback whole: a tap must say which
/// row, in which tenant, for which athlete. Show full text is left off when
/// the athlete wrote nothing.
fn feedback_buttons(fields: &HashMap<String, String>) -> Option<Vec<Value>> {
    let uuid = |name: &str| fields.get(name).and_then(|raw| Uuid::parse_str(raw).ok());
    let feedback = FeedbackRef {
        kind: FeedbackKind::of_source(fields.get("source")?)?,
        tenant_id: uuid("tenant_id")?,
        user_id: uuid("user_id")?,
        id: uuid("feedback_id")?,
    }
    .encode();
    let mut buttons = vec![json!({
        "type": "button",
        "action_id": format!("{FEEDBACK_REPLY_PREFIX}{feedback}"),
        "text": { "type": "plain_text", "text": "Reply" },
        "style": "primary",
    })];
    let wrote_something = fields
        .get("comment_len")
        .is_some_and(|len| len.parse::<usize>().is_ok_and(|len| len > 0));
    if wrote_something {
        buttons.push(json!({
            "type": "button",
            "action_id": format!("{FEEDBACK_TEXT_PREFIX}{feedback}"),
            "text": { "type": "plain_text", "text": "Show full text" },
        }));
    }
    Some(buttons)
}

/// Authorize an ops action by trusting the room `event` is routed to.
///
/// The room is the channel the contremaitre routing rule for `event` names,
/// the same rule that delivers the ping a tap answers, so the room is
/// configured in one place and a rerouted ping moves its decisions with it.
/// The action is honoured only when it originates there. Fails closed
/// (rejects) when the event has no rule of its own naming a channel (the
/// routing's default room is never trusted) or the channel does not match.
pub(super) fn verify_ops_channel(
    action: &SlackAction,
    event: &str,
    routing: &ContremaitreRoutingProvider,
) -> AppResult<()> {
    verify_ops_room(&action.channel_name, &action.channel_id, event, routing)
}

/// [`verify_ops_channel`] for a room named outside a tap: a reply modal's
/// submission carries the room its card was tapped in.
pub(super) fn verify_ops_room(
    channel_name: &str,
    channel_id: &str,
    event: &str,
    routing: &ContremaitreRoutingProvider,
) -> AppResult<()> {
    let configured = routing.event_channel(event).ok_or_else(|| {
        AppError::internal(format!("No notify routing rule names the room for {event}"))
    })?;

    if channel_matches(channel_name, channel_id, &configured) {
        return Ok(());
    }

    warn!(
        channel_id = %channel_id,
        channel_name = %channel_name,
        "Slack ops action rejected: not from the ops channel"
    );
    Err(AppError::auth_invalid(
        "This decision is only accepted from its ops channel",
    ))
}

/// Does an action's channel match the configured ops channel?
///
/// Compares the configured channel (a name like `#dravr-dev-users` or a raw
/// channel ID) against the action's channel name and ID. The leading `#` is
/// optional. An empty configured value never matches, keeping the check
/// fail-closed.
#[must_use]
pub fn channel_matches(channel_name: &str, channel_id: &str, configured: &str) -> bool {
    let expected = configured.trim().trim_start_matches('#');
    !expected.is_empty() && (channel_name == expected || channel_id == expected)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use uuid::Uuid;

    use dravr_contremaitre::config::NOTIFY_ROUTING_YAML;
    use dravr_tronc::notify::{NotifyActions, RoutingProvider};

    use super::super::feedback_reply_tap::{FeedbackKind, FeedbackRef, FeedbackTap};
    use super::super::ActionRoute;
    use super::{
        parse_action_id, verify_ops_channel, verify_ops_room, ActionType, CoachAccessDecision,
        ContremaitreRoutingProvider, OpsAction, OpsDecisionActions, SlackAction,
        COACH_ACCESS_REQUESTED_EVENT, FEEDBACK_RECORDED_EVENT, PENDING_APPROVAL_EVENT,
        QUOTA_LIMIT_REACHED_EVENT,
    };

    /// The `action_id`s of the buttons posted under `event`.
    fn button_action_ids(event: &str, fields: &HashMap<String, String>) -> Option<Vec<String>> {
        OpsDecisionActions
            .actions_for(event, fields)
            .map(|buttons| {
                buttons
                    .iter()
                    .map(|button| button["action_id"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
    }

    /// Each button posted under `event` as the action it parses back to, its
    /// block type, style and label.
    fn buttons_under(
        event: &str,
        fields: &HashMap<String, String>,
    ) -> Vec<(OpsAction, String, String, String)> {
        OpsDecisionActions
            .actions_for(event, fields)
            .unwrap()
            .iter()
            .map(|button| {
                (
                    parse_action_id(button["action_id"].as_str().unwrap()).unwrap(),
                    button["type"].as_str().unwrap().to_owned(),
                    button["style"].as_str().unwrap().to_owned(),
                    button["text"]["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn owned(button: (OpsAction, &str, &str, &str)) -> (OpsAction, String, String, String) {
        let (action, kind, style, label) = button;
        (action, kind.to_owned(), style.to_owned(), label.to_owned())
    }

    #[test]
    fn approve_and_reject_act_on_the_waiting_account_from_its_room() {
        let user_id = Uuid::new_v4();
        let fields = HashMap::from([("user_id".to_owned(), user_id.to_string())]);
        assert_eq!(
            buttons_under(PENDING_APPROVAL_EVENT, &fields),
            [
                (
                    OpsAction::SignUp(ActionType::Approve, user_id),
                    "button",
                    "primary",
                    "Approve"
                ),
                (
                    OpsAction::SignUp(ActionType::Reject, user_id),
                    "button",
                    "danger",
                    "Reject"
                ),
            ]
            .map(owned)
        );
        for id in button_action_ids(PENDING_APPROVAL_EVENT, &fields).unwrap() {
            assert_eq!(ActionRoute::of(&id), ActionRoute::Ops, "{id}");
            let action = parse_action_id(&id).unwrap();
            assert_eq!(action.event(), PENDING_APPROVAL_EVENT, "{id}");
        }
    }

    #[test]
    fn grant_and_decline_act_on_the_coach_access_request_not_its_requester() {
        let request_id = Uuid::new_v4();
        // The notice names the requester too; the buttons must not act on them.
        let fields = HashMap::from([
            ("request_id".to_owned(), request_id.to_string()),
            ("user_id".to_owned(), Uuid::new_v4().to_string()),
        ]);
        assert_eq!(
            buttons_under(COACH_ACCESS_REQUESTED_EVENT, &fields),
            [
                (
                    OpsAction::CoachAccess(CoachAccessDecision::Grant, request_id),
                    "button",
                    "primary",
                    "Grant"
                ),
                (
                    OpsAction::CoachAccess(CoachAccessDecision::Decline, request_id),
                    "button",
                    "danger",
                    "Decline"
                ),
            ]
            .map(owned)
        );
        for id in button_action_ids(COACH_ACCESS_REQUESTED_EVENT, &fields).unwrap() {
            assert_eq!(ActionRoute::of(&id), ActionRoute::Ops, "{id}");
            let action = parse_action_id(&id).unwrap();
            assert_eq!(action.event(), COACH_ACCESS_REQUESTED_EVENT, "{id}");
        }
    }

    #[test]
    fn raise_to_enterprise_is_the_one_button_under_a_reached_cap_and_acts_on_the_athlete() {
        let user_id = Uuid::new_v4();
        let fields = HashMap::from([
            ("user_id".to_owned(), user_id.to_string()),
            ("tenant_id".to_owned(), Uuid::new_v4().to_string()),
            ("limit_type".to_owned(), "daily_messages".to_owned()),
            ("tier".to_owned(), "starter".to_owned()),
            ("limit".to_owned(), "50".to_owned()),
        ]);
        assert_eq!(
            buttons_under(QUOTA_LIMIT_REACHED_EVENT, &fields),
            [(
                OpsAction::RaiseToEnterprise(user_id),
                "button",
                "primary",
                "Raise to Enterprise"
            )]
            .map(owned),
            "a reached cap offers exactly one unlock, never a danger button"
        );
        assert_eq!(
            button_action_ids(QUOTA_LIMIT_REACHED_EVENT, &fields),
            Some(vec![format!("raise_tier_enterprise:{user_id}")]),
            "the button's action_id is the raise prefix followed by the athlete"
        );
        for id in button_action_ids(QUOTA_LIMIT_REACHED_EVENT, &fields).unwrap() {
            assert_eq!(ActionRoute::of(&id), ActionRoute::Ops, "{id}");
            let action = parse_action_id(&id).unwrap();
            assert_eq!(action.event(), QUOTA_LIMIT_REACHED_EVENT, "{id}");
        }
    }

    #[test]
    fn no_buttons_for_another_event_or_an_event_naming_nothing_to_act_on() {
        let fields = HashMap::from([
            ("user_id".to_owned(), Uuid::new_v4().to_string()),
            ("request_id".to_owned(), Uuid::new_v4().to_string()),
        ]);
        for other in [
            "user.signed_up",
            "coach_access.granted",
            "coach_access.declined",
        ] {
            assert_eq!(button_action_ids(other, &fields), None, "{other}");
        }
        assert_eq!(
            button_action_ids(PENDING_APPROVAL_EVENT, &HashMap::new()),
            None
        );
        // A reached-cap notice that lost its athlete has nobody to raise.
        let no_athlete = HashMap::from([("limit_type".to_owned(), "weekly_tokens".to_owned())]);
        assert_eq!(
            button_action_ids(QUOTA_LIMIT_REACHED_EVENT, &no_athlete),
            None,
            "no user_id, no Raise to Enterprise button"
        );
        // A coach-access notice without its request names only the requester.
        let requester_only = HashMap::from([("user_id".to_owned(), Uuid::new_v4().to_string())]);
        assert_eq!(
            button_action_ids(COACH_ACCESS_REQUESTED_EVENT, &requester_only),
            None
        );
    }

    #[test]
    fn an_ops_action_naming_no_id_or_no_known_decision_is_refused() {
        for unknown in [
            "grant_coach_access:not-a-uuid",
            "approve_user:",
            "raise_tier_enterprise:not-a-uuid",
            "raise_tier_enterprise:",
            "promote_user:6f1c2b9e-0000-4000-8000-000000000001",
            "feedback_reply:thread:6f1c2b9e-0000-4000-8000-000000000001",
            "feedback_text:poll:6f1c2b9e-0000-4000-8000-000000000001:6f1c2b9e-0000-4000-8000-000000000001:6f1c2b9e-0000-4000-8000-000000000001",
        ] {
            assert!(parse_action_id(unknown).is_err(), "{unknown}");
        }
    }

    /// The fields a `feedback.recorded` event carries for one piece of
    /// feedback given as `source`, with `comment_len` characters of text.
    fn feedback_fields(source: &str, comment_len: usize) -> (HashMap<String, String>, [Uuid; 3]) {
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let fields = HashMap::from([
            ("tenant_id".to_owned(), ids[0].to_string()),
            ("user_id".to_owned(), ids[1].to_string()),
            ("feedback_id".to_owned(), ids[2].to_string()),
            ("source".to_owned(), source.to_owned()),
            ("comment_len".to_owned(), comment_len.to_string()),
        ]);
        (fields, ids)
    }

    #[test]
    fn feedback_is_answered_with_reply_and_read_whole_with_show_full_text_from_its_room() {
        for (source, kind) in [
            ("thread", FeedbackKind::Thread),
            ("thumbs", FeedbackKind::Rating),
            ("reaction", FeedbackKind::Rating),
        ] {
            let (fields, [tenant_id, user_id, id]) = feedback_fields(source, 37);
            let feedback = FeedbackRef {
                kind,
                tenant_id,
                user_id,
                id,
            };
            let buttons = OpsDecisionActions
                .actions_for(FEEDBACK_RECORDED_EVENT, &fields)
                .expect("feedback gets its buttons");
            let labels: Vec<&str> = buttons
                .iter()
                .map(|b| b["text"]["text"].as_str().unwrap())
                .collect();
            assert_eq!(labels, ["Reply", "Show full text"], "{source}");
            assert_eq!(buttons[0]["style"], "primary", "{source}");
            assert!(
                buttons[1].get("style").is_none(),
                "reading is no decision, so it is not styled: {source}"
            );
            let actions: Vec<OpsAction> = buttons
                .iter()
                .map(|b| parse_action_id(b["action_id"].as_str().unwrap()).unwrap())
                .collect();
            assert_eq!(
                actions,
                [
                    OpsAction::Feedback(FeedbackTap::Reply, feedback),
                    OpsAction::Feedback(FeedbackTap::FullText, feedback),
                ],
                "each button names the feedback by kind, tenant, athlete and row: {source}"
            );
            for button in &buttons {
                let id = button["action_id"].as_str().unwrap();
                assert!(id.len() <= 255, "Slack caps an action_id at 255: {id}");
                assert_eq!(ActionRoute::of(id), ActionRoute::Ops, "{id}");
                assert_eq!(
                    parse_action_id(id).unwrap().event(),
                    FEEDBACK_RECORDED_EVENT,
                    "{id}"
                );
            }
        }
    }

    #[test]
    fn feedback_without_words_offers_no_full_text_and_unnamed_feedback_no_buttons() {
        let (silent, _) = feedback_fields("thumbs", 0);
        let labels: Vec<String> = OpsDecisionActions
            .actions_for(FEEDBACK_RECORDED_EVENT, &silent)
            .unwrap()
            .iter()
            .map(|b| b["text"]["text"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(labels, ["Reply"], "a bare thumb has no text to show");

        for missing in ["tenant_id", "user_id", "feedback_id", "source"] {
            let (mut fields, _) = feedback_fields("thread", 12);
            fields.remove(missing);
            assert_eq!(
                button_action_ids(FEEDBACK_RECORDED_EVENT, &fields),
                None,
                "no {missing}, nothing a tap could act on"
            );
        }
        let (mut unknown_source, _) = feedback_fields("thread", 12);
        unknown_source.insert("source".to_owned(), "survey".to_owned());
        assert_eq!(
            button_action_ids(FEEDBACK_RECORDED_EVENT, &unknown_source),
            None,
            "bug: a source no row is stored under names nothing a tap could find"
        );
    }

    #[test]
    fn a_reply_is_accepted_only_from_the_feedback_room() {
        let routing = ContremaitreRoutingProvider::new_empty();
        routing
            .reload(
                "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
                 feedback.recorded:\n    channel: \"C0FEEDBACK\"\n  \
                 user.pending_approval:\n    channel: \"C0SIGNUPS\"\n",
            )
            .expect("a two-room table parses");
        assert!(
            verify_ops_room("", "C0FEEDBACK", FEEDBACK_RECORDED_EVENT, &routing).is_ok(),
            "bug: a reply from the room feedback.recorded is routed to must be honoured"
        );
        for elsewhere in ["C0SIGNUPS", "C0DEFAULT", ""] {
            assert!(
                verify_ops_room("", elsewhere, FEEDBACK_RECORDED_EVENT, &routing).is_err(),
                "a reply carried from {elsewhere:?} is refused"
            );
        }
    }

    fn tap_in(channel_id: &str, channel_name: &str) -> SlackAction {
        SlackAction {
            action_id: "approve_user:00000000-0000-0000-0000-000000000001".to_owned(),
            slack_user_id: "U1".to_owned(),
            slack_username: "operator".to_owned(),
            channel_id: channel_id.to_owned(),
            channel_name: channel_name.to_owned(),
            message_ts: "1.0".to_owned(),
            message_blocks: Vec::new(),
            trigger_id: String::new(),
        }
    }

    #[test]
    fn each_decision_is_honoured_only_in_the_room_its_own_event_is_routed_to() {
        let routing = ContremaitreRoutingProvider::new_empty();
        routing
            .reload(
                "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
                 user.pending_approval:\n    channel: \"C0SIGNUPS\"\n  \
                 coach_access.requested:\n    channel: \"C0COACHES\"\n  \
                 quota.limit_reached:\n    channel: \"C0QUOTA\"\n",
            )
            .expect("a three-room table parses");

        let grant = OpsAction::CoachAccess(CoachAccessDecision::Grant, Uuid::new_v4()).event();
        let approve = OpsAction::SignUp(ActionType::Approve, Uuid::new_v4()).event();
        assert!(verify_ops_channel(&tap_in("C0COACHES", ""), grant, &routing).is_ok());
        assert!(verify_ops_channel(&tap_in("C0SIGNUPS", ""), approve, &routing).is_ok());

        // The sign-up room does not decide coach access, nor the reverse.
        let err = verify_ops_channel(&tap_in("C0SIGNUPS", ""), grant, &routing)
            .expect_err("a grant from the sign-up room is refused");
        assert!(err.to_string().contains("ops channel"), "{err}");
        assert!(verify_ops_channel(&tap_in("C0COACHES", ""), approve, &routing).is_err());

        // A raise is honoured in the quota room and nowhere else.
        let raise = OpsAction::RaiseToEnterprise(Uuid::new_v4()).event();
        assert!(verify_ops_channel(&tap_in("C0QUOTA", ""), raise, &routing).is_ok());
        for elsewhere in ["C0SIGNUPS", "C0COACHES", "C0DEFAULT"] {
            assert!(
                verify_ops_channel(&tap_in(elsewhere, ""), raise, &routing).is_err(),
                "a raise from {elsewhere} must be refused"
            );
        }
        // Nor is the quota room where sign-ups are decided.
        assert!(verify_ops_channel(&tap_in("C0QUOTA", ""), approve, &routing).is_err());

        // A table naming only the sign-up room leaves coach access nowhere.
        routing
            .reload(
                "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
                 user.pending_approval:\n    channel: \"C0SIGNUPS\"\n",
            )
            .expect("a one-room table parses");
        let err = verify_ops_channel(&tap_in("C0SIGNUPS", ""), grant, &routing)
            .expect_err("no rule of its own, no grants");
        assert!(
            err.to_string().contains(COACH_ACCESS_REQUESTED_EVENT),
            "{err}"
        );
    }

    #[test]
    fn the_pinned_routing_names_a_room_of_its_own_for_coach_access_requests() {
        let routing = ContremaitreRoutingProvider::new_empty();
        routing
            .reload(NOTIFY_ROUTING_YAML)
            .expect("the pinned routing rules parse");
        let room = routing
            .event_channel(COACH_ACCESS_REQUESTED_EVENT)
            .expect("the pinned routing names the coach-access room");
        let firehose = RoutingProvider::route_for(&routing, "test.unrouted")
            .expect("the pinned routing has a default room")
            .channel;
        assert_ne!(
            room, firehose,
            "coach access is decided in a room of its own"
        );
    }

    #[test]
    fn an_approval_is_honoured_only_in_the_room_the_pending_ping_is_routed_to() {
        let routing = ContremaitreRoutingProvider::new_empty();
        routing
            .reload(NOTIFY_ROUTING_YAML)
            .expect("the pinned routing rules parse");

        // Both rooms come from the pinned table, so a reroute upstream moves
        // the test with it.
        let room = routing
            .event_channel("user.pending_approval")
            .expect("the pinned routing names the approvals room");
        let firehose = RoutingProvider::route_for(&routing, "test.unrouted")
            .expect("the pinned routing has a default room")
            .channel;
        assert_ne!(room, firehose, "approvals have a room of their own");

        assert!(verify_ops_channel(&tap_in(&room, ""), PENDING_APPROVAL_EVENT, &routing).is_ok());
        // The default room carries other events, never an approval.
        let err = verify_ops_channel(&tap_in(&firehose, ""), PENDING_APPROVAL_EVENT, &routing)
            .expect_err("a tap outside the approvals room is refused");
        assert!(err.to_string().contains("ops channel"), "{err}");
    }

    #[test]
    fn an_approval_is_refused_while_no_rule_of_its_own_names_the_room() {
        let routing = ContremaitreRoutingProvider::new_empty();
        let err = verify_ops_channel(
            &tap_in("C0ANS7HU3CH", "dravr-dev-users"),
            PENDING_APPROVAL_EVENT,
            &routing,
        )
        .expect_err("no routing rule, no approvals");
        assert!(err.to_string().contains("user.pending_approval"), "{err}");

        // A table without the event's rule still routes it to the default
        // room; that room is never where approvals are honoured.
        routing
            .reload("version: 1\ndefaults:\n  channel: \"C0B3FRDDD25\"\nevents: {}\n")
            .expect("a defaults-only table parses");
        let err = verify_ops_channel(
            &tap_in("C0B3FRDDD25", "dev-dravr-events"),
            PENDING_APPROVAL_EVENT,
            &routing,
        )
        .expect_err("the default room is not the approvals room");
        assert!(err.to_string().contains("user.pending_approval"), "{err}");
    }
}
