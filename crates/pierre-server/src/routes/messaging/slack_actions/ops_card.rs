// ABOUTME: What the super-admin-gated ops taps share: the super-admin check behind a Slack identity and the card they redraw
// ABOUTME: A settled tap trades the card's buttons for its outcome; a refused one keeps them and says why under them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use super::{authorize_postback, update_card, SlackAction};
use crate::mcp::resources::ServerContext;

/// What the room is told about `e`: a refusal or a settled request in its own
/// words, which name what to do next, and a server fault only by its kind,
/// since a database or internal message is no one's to read in Slack.
pub(super) fn room_text(e: &AppError) -> &str {
    if e.is_server_fault() {
        e.code.description()
    } else {
        &e.message
    }
}

/// The decision a super-admin-gated tap takes, which names what the room is
/// told when the tapper is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SuperAdminDecision {
    /// Grant or decline a coach-access request.
    CoachAccess,
    /// Raise a capped athlete to Enterprise.
    RaiseTier,
}

impl SuperAdminDecision {
    /// The refusal for a tapper who is not an active super-admin. Each message
    /// is a literal so the `PermissionDenied` review reads it.
    fn refusal(self) -> AppError {
        match self {
            Self::CoachAccess => AppError::new(
                ErrorCode::PermissionDenied,
                "Coach access is decided only by an active super-admin",
            ),
            Self::RaiseTier => AppError::new(
                ErrorCode::PermissionDenied,
                "A tier is raised only by an active super-admin",
            ),
        }
    }
}

/// The Dravr account a Slack identity acts as on a super-admin's decision:
/// the one its channel link names, through the gate every command postback
/// walks ([`authorize_postback`]), and only when it is an active super-admin.
/// `decision` names what the room is told when the account is not one.
///
/// # Errors
///
/// Refuses an unlinked Slack identity, a pending or suspended account, and
/// an account that is not an active super-admin.
pub(super) async fn authorize_super_admin(
    resources: &ServerContext,
    slack_user_id: &str,
    decision: SuperAdminDecision,
) -> AppResult<Uuid> {
    let (user_id, _) = authorize_postback(resources, slack_user_id).await?;
    let is_super_admin = resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await?
        .is_some_and(|user| user.is_active_super_admin());
    if is_super_admin {
        return Ok(user_id);
    }
    warn!(
        slack_user = %slack_user_id,
        user_id = %user_id,
        "Slack ops action refused: not an active super-admin"
    );
    Err(decision.refusal())
}

/// `block_id` of the line a refused tap adds under a card's buttons; the next
/// refusal replaces it rather than stacking under it.
const REFUSAL_BLOCK_ID: &str = "ops-refusal";

/// The card a settled request leaves: the notice it was posted with, its
/// buttons (and any refusal under them) replaced by `outcome`.
pub(super) fn settled_card(posted: &[Value], outcome: &str) -> Value {
    let mut blocks: Vec<Value> = posted
        .iter()
        .filter(|block| {
            block.get("type").and_then(Value::as_str) != Some("actions") && !is_refusal(block)
        })
        .cloned()
        .collect();
    blocks.push(json!({
        "type": "section",
        "text": { "type": "mrkdwn", "text": outcome }
    }));
    Value::Array(blocks)
}

/// The card a refused tap leaves: as posted, buttons and all, with `reason`
/// under them in place of any earlier refusal. `None` when the payload carried
/// no card to keep, since a card redrawn from the reason alone would take the
/// buttons away from whoever may still decide.
fn refused_card(posted: &[Value], reason: &str) -> Option<Value> {
    if posted.is_empty() {
        return None;
    }
    let mut blocks: Vec<Value> = posted
        .iter()
        .filter(|block| !is_refusal(block))
        .cloned()
        .collect();
    blocks.push(json!({
        "type": "context",
        "block_id": REFUSAL_BLOCK_ID,
        "elements": [{ "type": "mrkdwn", "text": format!(":no_entry: {reason}") }]
    }));
    Some(Value::Array(blocks))
}

fn is_refusal(block: &Value) -> bool {
    block.get("block_id").and_then(Value::as_str) == Some(REFUSAL_BLOCK_ID)
}

/// Say why a tap was not honoured under the card's buttons, keeping them.
pub(super) fn keep_card_with_reason(resources: &ServerContext, action: &SlackAction, reason: &str) {
    if let Some(card) = refused_card(&action.message_blocks, reason) {
        update_card(resources, action, &card);
    } else {
        warn!(
            channel_id = %action.channel_id,
            "Slack action payload carried no card blocks; the refusal was not shown"
        );
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::errors::{AppError, ErrorCode};
    use serde_json::{json, Value};

    use super::{refused_card, room_text, settled_card, REFUSAL_BLOCK_ID};

    #[test]
    fn the_room_reads_a_refusal_in_its_own_words_and_a_server_fault_only_by_kind() {
        let refused = AppError::new(
            ErrorCode::PermissionDenied,
            "Coach access is decided only by an active super-admin",
        );
        assert_eq!(
            room_text(&refused),
            "Coach access is decided only by an active super-admin"
        );
        let decided = AppError::new(
            ErrorCode::ResourceAlreadyExists,
            "This coach access request was already decided",
        );
        assert_eq!(
            room_text(&decided),
            "This coach access request was already decided"
        );

        let fault = AppError::database("relation coach_access_requests: connection reset");
        assert_eq!(room_text(&fault), fault.code.description());
        assert!(!room_text(&fault).contains("connection reset"));
    }

    /// A card as the notify layer posts it: the notice, then its buttons.
    fn posted_card() -> Vec<Value> {
        vec![
            json!({ "type": "section", "block_id": "notice",
                    "text": { "type": "mrkdwn", "text": "coach access requested" } }),
            json!({ "type": "context", "block_id": "who",
                    "elements": [{ "type": "mrkdwn", "text": "coach@example.test" }] }),
            json!({ "type": "actions", "block_id": "notify-actions-coach_access.requested",
                    "elements": [{ "type": "button", "action_id": "grant_coach_access:1" }] }),
        ]
    }

    fn block_ids(card: &Value) -> Vec<&str> {
        card.as_array()
            .unwrap()
            .iter()
            .map(|block| block["block_id"].as_str().unwrap_or("-"))
            .collect()
    }

    #[test]
    fn a_settled_request_keeps_its_notice_and_trades_its_buttons_for_the_outcome() {
        let refused = refused_card(&posted_card(), "@op: not a super-admin").unwrap();
        let card = settled_card(refused.as_array().unwrap(), ":white_check_mark: granted");

        assert_eq!(block_ids(&card), ["notice", "who", "-"]);
        assert_eq!(card[2]["text"]["text"], ":white_check_mark: granted");
    }

    #[test]
    fn a_refused_tap_keeps_the_buttons_and_says_why_once_under_them() {
        let first = refused_card(&posted_card(), "@op: not linked").unwrap();
        let second = refused_card(first.as_array().unwrap(), "@admin: not a super-admin").unwrap();

        assert_eq!(
            block_ids(&second),
            [
                "notice",
                "who",
                "notify-actions-coach_access.requested",
                REFUSAL_BLOCK_ID
            ]
        );
        assert_eq!(
            second[3]["elements"][0]["text"],
            ":no_entry: @admin: not a super-admin"
        );
    }

    #[test]
    fn a_refusal_without_the_posted_card_redraws_nothing() {
        assert_eq!(refused_card(&[], "@op: not linked"), None);
    }
}
