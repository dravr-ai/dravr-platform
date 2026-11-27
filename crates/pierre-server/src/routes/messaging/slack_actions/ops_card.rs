// ABOUTME: What the admin-gated ops taps share: the admin check behind a Slack identity and the card they redraw
// ABOUTME: A settled tap trades the card's buttons for its outcome; a refused one keeps them and says why under them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::User;
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use super::{authorize_postback, update_card, SlackAction, SLACK_CHANNEL};
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

/// The decision an admin-gated tap takes, which names what the room is
/// told when the tapper is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdminDecision {
    /// Grant or decline a coach-access request.
    CoachAccess,
    /// Raise a capped athlete to Enterprise.
    RaiseTier,
}

impl AdminDecision {
    /// The refusal for a tapper who is not an active admin. Each message
    /// is a literal so the `PermissionDenied` review reads it.
    fn refusal(self) -> AppError {
        match self {
            Self::CoachAccess => AppError::new(
                ErrorCode::PermissionDenied,
                "Coach access is decided only by an active admin",
            ),
            Self::RaiseTier => AppError::new(
                ErrorCode::PermissionDenied,
                "A tier is raised only by an active admin",
            ),
        }
    }
}

/// A refused admin-gated tap: the error the route answers with, and the Dravr
/// account the tapper's Slack identity acts as, when one is linked.
///
/// The account stays out of the error, whose message is a reviewed literal
/// the route also answers with; only the room line under the card names it,
/// so whoever was refused can tell a wrong link from a wrong role.
#[derive(Debug)]
pub(super) struct AdminRefusal {
    /// What the route answers with.
    pub(super) error: AppError,
    /// The linked account, as [`acting_as`] words it.
    account: Option<String>,
}

impl AdminRefusal {
    /// The line the room reads under the card for `tapper`.
    pub(super) fn room_line(&self, tapper: &str) -> String {
        let reason = room_text(&self.error);
        self.account.as_ref().map_or_else(
            || format!("{tapper}: {reason}"),
            |account| format!("{tapper}: {reason}. This Slack account acts as {account}."),
        )
    }
}

/// `user` as a refusal names it: email, role, and whether it may act now.
/// The ops room is the team's private channel, so the email is shown whole.
fn acting_as(user: &User) -> String {
    let standing = if user.is_active {
        user.user_status.to_string()
    } else {
        "deactivated".to_owned()
    };
    format!("{} (role {}, {standing})", user.email, user.role)
}

/// The account `slack_user_id`'s channel link names, worded by [`acting_as`],
/// for a refusal that came before the gate read it: a pending or suspended
/// account. `None` when no link or account exists; a failed read is logged
/// and leaves the refusal without the account, never changes it.
async fn linked_account(resources: &ServerContext, slack_user_id: &str) -> Option<String> {
    let repos = &resources.common.repos;
    let lookup = async {
        let Some(tenant_id) = repos
            .messaging
            .get_channel_link_tenant(SLACK_CHANNEL, slack_user_id)
            .await?
        else {
            return Ok(None);
        };
        let Some(link) = repos
            .messaging
            .get_channel_link(tenant_id, SLACK_CHANNEL, slack_user_id)
            .await?
        else {
            return Ok(None);
        };
        let Some(user_id) = link["user_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
        else {
            return Ok(None);
        };
        Ok::<_, AppError>(
            repos
                .users
                .get_global(user_id)
                .await?
                .as_ref()
                .map(acting_as),
        )
    };
    lookup.await.unwrap_or_else(|e| {
        warn!(
            slack_user = %slack_user_id,
            error = %e,
            "Could not name the account a refused Slack ops tap acts as"
        );
        None
    })
}

/// The Dravr account a Slack identity acts as on an admin's decision:
/// the one its channel link names, through the gate every command postback
/// walks ([`authorize_postback`]), and only when it is an active admin.
/// `decision` names what the room is told when the account is not one.
///
/// # Errors
///
/// Refuses an unlinked Slack identity, a pending or suspended account, and
/// an account that is not an active admin, naming the linked account when
/// there is one.
pub(super) async fn authorize_admin(
    resources: &ServerContext,
    slack_user_id: &str,
    decision: AdminDecision,
) -> Result<Uuid, AdminRefusal> {
    let user_id = match authorize_postback(resources, slack_user_id).await {
        Ok(caller) => caller.user_id,
        Err(error) => {
            let account = linked_account(resources, slack_user_id).await;
            return Err(AdminRefusal { error, account });
        }
    };
    let user = resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .map_err(|error| AdminRefusal {
            error,
            account: None,
        })?;
    if user.as_ref().is_some_and(User::is_active_admin) {
        return Ok(user_id);
    }
    warn!(
        slack_user = %slack_user_id,
        user_id = %user_id,
        "Slack ops action refused: not an active admin"
    );
    Err(AdminRefusal {
        error: decision.refusal(),
        account: user.as_ref().map(acting_as),
    })
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

/// Whether `block` is the line a refused tap added under a card's buttons.
pub(super) fn is_refusal(block: &Value) -> bool {
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
    use pierre_core::models::{User, UserStatus};
    use pierre_core::permissions::UserRole;
    use serde_json::{json, Value};

    use super::{
        acting_as, refused_card, room_text, settled_card, AdminDecision, AdminRefusal,
        REFUSAL_BLOCK_ID,
    };

    fn account(role: UserRole, status: UserStatus, is_active: bool) -> User {
        let mut user = User::new("jf@dravr.ai".to_owned(), "unused".to_owned(), None);
        user.role = role;
        user.user_status = status;
        user.is_active = is_active;
        user
    }

    #[test]
    fn a_refusal_names_the_account_the_slack_identity_acts_as() {
        let refusal = AdminRefusal {
            error: AdminDecision::RaiseTier.refusal(),
            account: Some(acting_as(&account(
                UserRole::User,
                UserStatus::Active,
                true,
            ))),
        };
        assert_eq!(
            refusal.room_line("@jf"),
            "@jf: A tier is raised only by an active admin. \
             This Slack account acts as jf@dravr.ai (role user, active)."
        );
    }

    #[test]
    fn the_account_reads_as_suspended_or_deactivated_whatever_its_role() {
        assert_eq!(
            acting_as(&account(UserRole::Admin, UserStatus::Suspended, true)),
            "jf@dravr.ai (role admin, suspended)"
        );
        assert_eq!(
            acting_as(&account(UserRole::Admin, UserStatus::Active, false)),
            "jf@dravr.ai (role admin, deactivated)"
        );
    }

    #[test]
    fn a_refusal_with_no_linked_account_reads_as_its_reason_alone() {
        let refusal = AdminRefusal {
            error: AdminDecision::CoachAccess.refusal(),
            account: None,
        };
        assert_eq!(
            refusal.room_line("@op"),
            "@op: Coach access is decided only by an active admin"
        );
    }

    #[test]
    fn the_room_reads_a_refusal_in_its_own_words_and_a_server_fault_only_by_kind() {
        let refused = AppError::new(
            ErrorCode::PermissionDenied,
            "Coach access is decided only by an active admin",
        );
        assert_eq!(
            room_text(&refused),
            "Coach access is decided only by an active admin"
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
        let refused = refused_card(&posted_card(), "@op: not an admin").unwrap();
        let card = settled_card(refused.as_array().unwrap(), ":white_check_mark: granted");

        assert_eq!(block_ids(&card), ["notice", "who", "-"]);
        assert_eq!(card[2]["text"]["text"], ":white_check_mark: granted");
    }

    #[test]
    fn a_refused_tap_keeps_the_buttons_and_says_why_once_under_them() {
        let first = refused_card(&posted_card(), "@op: not linked").unwrap();
        let second = refused_card(first.as_array().unwrap(), "@admin: not an admin").unwrap();

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
            ":no_entry: @admin: not an admin"
        );
    }

    #[test]
    fn a_refusal_without_the_posted_card_redraws_nothing() {
        assert_eq!(refused_card(&[], "@op: not linked"), None);
    }
}
