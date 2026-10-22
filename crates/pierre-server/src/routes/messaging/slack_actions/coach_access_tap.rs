// ABOUTME: Grant or decline a coach-access request from its Slack notice, for a linked active super-admin
// ABOUTME: Settles the card on a decision; a refused tap keeps the buttons and says why under them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::StatusCode;
use axum::Json;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_services::coach_access;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use super::{authorize_postback, update_card, CoachAccessDecision, SlackAction};
use crate::mcp::resources::ServerContext;

/// Grant or decline the coach-access request a tap names, for the
/// super-admin behind the Slack identity that tapped.
///
/// The room is not enough here: a grant gives `manages_roster`, which only a
/// super-admin decides (the console's coach-access routes are super-admin
/// only), so the clicker must also resolve to an active super-admin
/// ([`authorize_super_admin`]). The decision goes through
/// [`coach_access::grant`] / [`coach_access::decline`] with that admin as
/// `decided_by`, as the console's does, so `coach_access.granted` /
/// `.declined` fire the same way.
///
/// A settled request — decided now, decided before, or gone — turns the
/// card's buttons into the outcome. A refused tap, or a decision that failed,
/// keeps the buttons for whoever may still decide and says why under them.
/// Anything but a decision taken now is also the route's answer, so the
/// route logs it.
pub(super) async fn answer_coach_access_tap(
    resources: &ServerContext,
    action: &SlackAction,
    decision: CoachAccessDecision,
    request_id: Uuid,
) -> AppResult<(StatusCode, Json<Value>)> {
    let decider = format!("@{}", action.slack_username);

    let operator = match authorize_super_admin(resources, &action.slack_user_id).await {
        Ok(operator) => operator,
        Err(refused) => {
            keep_card_with_reason(
                resources,
                action,
                &format!("{decider}: {}", room_text(&refused)),
            );
            return Err(refused);
        }
    };

    match decide_coach_access(resources, decision, request_id, operator, &decider).await {
        Ok(outcome) => {
            info!(
                request_id = %request_id,
                operator_user_id = %operator,
                slack_user = %action.slack_user_id,
                "Coach access request decided via Slack action"
            );
            let card = settled_card(
                &action.message_blocks,
                &format!(":white_check_mark: {outcome}"),
            );
            update_card(resources, action, &card);
            Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
        }
        Err(e)
            if matches!(
                e.code,
                ErrorCode::ResourceAlreadyExists | ErrorCode::ResourceNotFound
            ) =>
        {
            let card = settled_card(&action.message_blocks, &format!(":x: {}", room_text(&e)));
            update_card(resources, action, &card);
            Err(e)
        }
        Err(e) => {
            keep_card_with_reason(
                resources,
                action,
                &format!(
                    "{decider}: the decision failed, try again ({})",
                    room_text(&e)
                ),
            );
            Err(e)
        }
    }
}

/// What the room is told about `e`: a refusal or a settled request in its own
/// words, which name what to do next, and a server fault only by its kind,
/// since a database or internal message is no one's to read in Slack.
fn room_text(e: &AppError) -> &str {
    if e.is_server_fault() {
        e.code.description()
    } else {
        &e.message
    }
}

/// Take `decision` on the request for `operator`, returning the outcome line
/// the card shows.
async fn decide_coach_access(
    resources: &ServerContext,
    decision: CoachAccessDecision,
    request_id: Uuid,
    operator: Uuid,
    decider: &str,
) -> AppResult<String> {
    let repos = &resources.common.repos;
    match decision {
        CoachAccessDecision::Grant => {
            let outcome = coach_access::grant(repos, request_id, Some(operator)).await?;
            Ok(if outcome.attached_group.is_some() {
                format!("Coach access granted by {decider}; they now coach their group")
            } else {
                format!("Coach access granted by {decider}")
            })
        }
        CoachAccessDecision::Decline => {
            coach_access::decline(repos, request_id, Some(operator)).await?;
            Ok(format!("Coach access declined by {decider}"))
        }
    }
}

/// The Dravr account a Slack identity decides coach access as: the one its
/// channel link names, through the gate every command postback walks
/// ([`authorize_postback`]), and only when it is an active super-admin.
///
/// # Errors
///
/// Refuses an unlinked Slack identity, a pending or suspended account, and
/// an account that is not an active super-admin.
async fn authorize_super_admin(resources: &ServerContext, slack_user_id: &str) -> AppResult<Uuid> {
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
        "Slack coach access action refused: not an active super-admin"
    );
    Err(AppError::new(
        ErrorCode::PermissionDenied,
        "Coach access is decided only by an active super-admin",
    ))
}

/// `block_id` of the line a refused tap adds under a card's buttons; the next
/// refusal replaces it rather than stacking under it.
const REFUSAL_BLOCK_ID: &str = "ops-refusal";

/// The card a settled request leaves: the notice it was posted with, its
/// buttons (and any refusal under them) replaced by `outcome`.
fn settled_card(posted: &[Value], outcome: &str) -> Value {
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
fn keep_card_with_reason(resources: &ServerContext, action: &SlackAction, reason: &str) {
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
