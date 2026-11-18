// ABOUTME: Approve or reject the account a sign-up notice names, from its Slack Approve/Reject buttons
// ABOUTME: The room authorizes sign-ups and no more: an admin account, or one no longer pending, is refused, and the card says why
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::StatusCode;
use axum::Json;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{User, UserStatus};
use pierre_middleware::mask_email;
use pierre_services::admin_ops::{
    self, AccountCaller, ActingOn, AdminStandingLost, SignUpApproval, SignUpDecision,
    StatusTransition,
};
use pierre_services::analytics::cache_user_email;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use super::ops_card::{keep_card_with_reason, room_text};
use super::{update_card, ActionType, SlackAction};
use crate::mcp::resources::ServerContext;
use crate::services::user_approval_notifier::ApprovalNotifier;

/// What a tap on an admin account's card is told.
const ADMIN_IS_NO_SIGN_UP: &str = "This account is an admin, and a sign-up card approves or \
     rejects sign-ups only. Nothing was changed; an admin is suspended or reinstated by another \
     admin, from the console.";

/// Approve or reject the account a sign-up tap names, then turn its card
/// into the outcome.
///
/// The room is the whole authorization here: whoever is in it approves and
/// rejects sign-ups without holding the admin role. That is all it
/// authorizes. A card is about a sign-up and an admin is never one, so a
/// tap that names an admin account, on a card left with live buttons, is
/// refused before anything is read or written further: it neither suspends
/// an admin nor sets a suspended one active again, which over the admin API
/// take an admin's console session or a super-admin token. The card loses
/// its buttons and says why.
///
/// A sign-up is a pending account, and a card decides nothing else
/// (carnet#1589). A card outlives its decision wherever its buttons are
/// still live, and the account it names may since have been approved,
/// suspended or approved again: a tap on it then changes nothing, so Reject
/// never suspends an active account and Approve never reinstates a
/// suspended one. The card says the account is no longer pending and what
/// its standing is. The write itself holds the rule
/// ([`admin_ops::decide_sign_up`], the service's one write for a surface
/// that decides sign-ups): it lands only while the account is still
/// pending, so of two taps that overlap, on one button or on both, the
/// first decides and the second changes nothing.
///
/// A tap that finds the account already where its button would put it,
/// Approve on an active account or Reject on a suspended one, is the same
/// decision arriving twice ([`read_tap`]). It changes nothing and redraws
/// nothing: the card of the first tap says who decided, and a second card
/// saying the account "is no longer pending" would replace that name.
///
/// A tap that read the sign-up pending and whose write then found it
/// decided lost to another tap, on either button, and draws nothing either
/// ([`TapOutcome::LostAtTheWrite`]). The two cards are drawn by two
/// detached calls to Slack, in no order: the loser's "no longer pending"
/// could land after the winner's card and replace who decided. Only a tap
/// that reads the account already decided says so on the card.
///
/// A rejection is the status write every other suspension makes. A pending
/// account that is not an admin can still hold what one did (a device login
/// or an impersonation left live by a build that demoted without
/// withdrawing them): the suspension withdraws those, on the trail under
/// the Slack source.
///
/// An approval gives the account a tenant of its own when it has none
/// ([`admin_ops::approve_user`]). When that fails after the approval
/// landed, the sign-up is put back to pending and the card keeps its
/// buttons, with what happened under them: the next tap, or an admin on
/// the console, approves it ([`TapOutcome::PutBack`]).
pub(super) async fn answer_sign_up_tap(
    resources: &ServerContext,
    action: &SlackAction,
    action_type: ActionType,
    user_uuid: Uuid,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Attribute the action to the Slack handle that clicked. The username
    // rides in the interactive payload, so no `users.info` call or
    // `users:read.email` scope is needed.
    let approver = format!("@{}", action.slack_username);

    // Execute the action
    let result = decide(resources, action_type, user_uuid, &approver).await;

    if let Some(text) = outcome_text(action_type, &result, &approver) {
        let blocks = json!([{
            "type": "section",
            "text": { "type": "mrkdwn", "text": text }
        }]);
        // Fire-and-forget message update via tronc SlackClient
        update_card(resources, action, &blocks);
    } else {
        leave_the_card(resources, action, &result, user_uuid, &approver);
    }

    // Return 200 immediately (Slack expects a response within 3 seconds)
    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

/// What a tap that replaces no card does instead ([`outcome_text`] is `None`).
///
/// An approval that was put back says so under the card's buttons, which
/// stay. A tap that lost at the write, or found the account already decided
/// that way, draws nothing. Each leaves one log line.
fn leave_the_card(
    resources: &ServerContext,
    action: &SlackAction,
    result: &AppResult<TapOutcome>,
    user_uuid: Uuid,
    approver: &str,
) {
    match result {
        Ok(TapOutcome::PutBack(cause)) => {
            info!(
                user_id = %user_uuid,
                tapped_by = %approver,
                "Sign-up approval was put back: the sign-up is pending again and the card keeps its buttons"
            );
            keep_card_with_reason(resources, action, &put_back_line(approver, cause));
        }
        Ok(TapOutcome::LostAtTheWrite) => info!(
            user_id = %user_uuid,
            tapped_by = %approver,
            "Sign-up tap lost at the write: the sign-up was decided first; the card is left as it is"
        ),
        _ => info!(
            user_id = %user_uuid,
            tapped_by = %approver,
            "Sign-up tap found the account already decided that way; the card is left as it is"
        ),
    }
}

/// What a tap came to.
#[derive(Debug, PartialEq, Eq)]
enum TapOutcome {
    /// The tap decided the sign-up: the account's email, for the card.
    Decided(String),
    /// The account already stood where the tap's button would put it:
    /// nothing was written, and the card stays as the deciding tap left it.
    AlreadyStands,
    /// The tap read a pending sign-up and its write found none: something
    /// decided the sign-up in between. Nothing was written, and the card
    /// stays as whatever decided it drew it.
    LostAtTheWrite,
    /// The approval landed, the account could not be given its tenant, and
    /// the sign-up is pending again: what failed, as the room may read it.
    /// The card keeps its buttons, for the tap that finishes.
    PutBack(String),
}

/// What the room reads under a card whose approval was put back.
fn put_back_line(approver: &str, cause: &str) -> String {
    format!(
        "{approver}: the approval did not complete ({cause}). The sign-up is pending again; \
         approve it again here, or from the console."
    )
}

/// What a tap finds when it reads the account it names.
#[derive(Debug, PartialEq, Eq)]
enum TapReading {
    /// A sign-up still waiting: the tap's decision is written.
    Pending,
    /// The account already stands where the tap's button would put it.
    AlreadyStands,
}

/// The status a button gives the sign-up it decides.
const fn decision_of(action_type: ActionType) -> UserStatus {
    match action_type {
        ActionType::Approve => UserStatus::Active,
        ActionType::Reject => UserStatus::Suspended,
    }
}

/// Read a tap that would write `decision` against the account it names,
/// before anything is written.
///
/// An account already in that status is the decision arriving twice, by a
/// second tap or the loser of two that overlap: nothing is left to do, and
/// that is asked first, whatever the account's role, since such a tap
/// changes nothing on any account. Every other account that is not a
/// pending sign-up is refused.
///
/// # Errors
///
/// Returns an invalid-input error for an admin account
/// ([`refuse_an_admin`]) and for one that is no longer pending
/// ([`refuse_unless_pending`]).
fn read_tap(user: &User, decision: UserStatus) -> AppResult<TapReading> {
    if user.user_status == decision {
        return Ok(TapReading::AlreadyStands);
    }
    refuse_an_admin(user)?;
    refuse_unless_pending(user)?;
    Ok(TapReading::Pending)
}

/// What the card becomes: the outcome and who tapped, or why nothing was
/// done. `None` leaves the card's notice and buttons as they are, for a tap
/// that found the account already where it would put it, for one that lost
/// at the write, and for an approval that was put back.
fn outcome_text(
    action_type: ActionType,
    result: &AppResult<TapOutcome>,
    approver: &str,
) -> Option<String> {
    match result {
        Ok(TapOutcome::AlreadyStands | TapOutcome::LostAtTheWrite | TapOutcome::PutBack(_)) => None,
        Ok(TapOutcome::Decided(email)) => Some(match action_type {
            ActionType::Approve => format!(":white_check_mark: *{email}* approved by {approver}"),
            ActionType::Reject => {
                format!(":white_check_mark: *{email}* rejected (suspended) by {approver}")
            }
        }),
        Err(e) => Some(format!(":x: Action failed: {e}")),
    }
}

/// Take the decision a tap names on the account it names: the account's
/// email when the tap decided the sign-up, that the account already stood
/// there, that another decision reached the write first, or that the
/// approval was put back.
///
/// The room acts as [`AccountCaller::slack_room`]: it decides sign-ups, is
/// written against no admin account, and may not act on an admin one. An
/// admin's card is refused in the card's own words before that
/// ([`read_tap`]), so the service's rank rule is the second lock on the
/// same door, never what the room reads.
///
/// # Errors
///
/// Returns a not-found error for an unknown account, an invalid-input error
/// for an admin account and for one that is no longer pending at this read
/// ([`read_tap`]), and whatever the approval or the rejection fails with.
async fn decide(
    resources: &ServerContext,
    action_type: ActionType,
    user_uuid: Uuid,
    tapped_by: &str,
) -> AppResult<TapOutcome> {
    let user = resources
        .common
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    if read_tap(&user, decision_of(action_type))? == TapReading::AlreadyStands {
        return Ok(TapOutcome::AlreadyStands);
    }
    let room = AccountCaller::slack_room(tapped_by);
    let acting = room.acting_on(user)?;
    match action_type {
        ActionType::Approve => approve_user(resources, &acting).await,
        ActionType::Reject => reject_user(resources, &acting).await,
    }
}

/// Refuse a tap whose account holds the admin role, whatever its status: a
/// sign-up card decides sign-ups, and the room's membership is not the rank
/// that suspends an admin or reinstates one.
fn refuse_an_admin(user: &User) -> AppResult<()> {
    if user.role.is_admin() {
        return Err(AppError::invalid_input(ADMIN_IS_NO_SIGN_UP));
    }
    Ok(())
}

/// Refuse a tap whose account is no longer waiting for a decision, saying
/// what its standing is: the card's sign-up was decided, here or on another
/// surface, and what happens to the account now is not a sign-up's decision.
fn refuse_unless_pending(user: &User) -> AppResult<()> {
    if user.user_status == UserStatus::Pending {
        return Ok(());
    }
    Err(AppError::invalid_input(format!(
        "This account is no longer pending: it is {}. A sign-up card approves or rejects \
         pending sign-ups only. Nothing was changed; an account that is not pending is \
         suspended or reinstated by an admin, from the console.",
        user.user_status
    )))
}

/// Approve a pending sign-up through the one approval every surface takes
/// ([`admin_ops::approve_user`]: for the room, active while still pending,
/// with a tenant of its own when it has none and its default MCP token, as
/// an admin's approval), then announce it.
///
/// A tap whose write found the sign-up decided draws nothing
/// ([`TapOutcome::LostAtTheWrite`]): whatever decided it draws its own card,
/// and a second card from here could land after it. That holds for the
/// three ways a write finds no pending row: the other button won, the same
/// button won, or the account left the queue another way (approved on the
/// console, deleted). A card left with live buttons answers its next tap
/// from that tap's own read.
async fn approve_user(resources: &ServerContext, acting: &ActingOn<'_>) -> AppResult<TapOutcome> {
    let repos = &resources.common.repos;
    let user_uuid = acting.id();
    let approved_by = acting.caller().name();
    let approval = match admin_ops::approve_user(repos, acting, None).await? {
        SignUpApproval::Approved(approval) => *approval,
        SignUpApproval::LostAtTheWrite => return Ok(TapOutcome::LostAtTheWrite),
        SignUpApproval::PutBack(cause) => {
            return Ok(TapOutcome::PutBack(room_text(&cause).to_owned()));
        }
    };
    let updated_user = approval.user;

    info!(
        user_id = %user_uuid,
        email = %mask_email(&updated_user.email),
        approved_by,
        "User approved via Slack action"
    );

    // Warm the identity cache first: the notify enricher resolves user_email
    // from user_id, and without it the Slack message loses the subject.
    cache_user_email(&user_uuid.to_string(), &updated_user.email);
    // The tenant user.approved is attributed to is the one sign-up keyed
    // its events on, so the two land on one identity.
    info!(
        target: "notify",
        event = "user.approved",
        user_id = %user_uuid,
        tenant_id = approval.tenant_id.map(display),
        approved_by = approved_by,
        "user account approved"
    );

    // Notify the approved user: account email + a message on each linked channel.
    ApprovalNotifier::from_context(resources)
        .notify_user_approved(
            user_uuid,
            &updated_user.email,
            updated_user.display_name.as_deref(),
        )
        .await;

    Ok(TapOutcome::Decided(updated_user.email))
}

/// Reject (suspend) a pending sign-up
async fn reject_user(resources: &ServerContext, acting: &ActingOn<'_>) -> AppResult<TapOutcome> {
    // The status write every suspension makes: what the account still held
    // from a time as an admin goes in the same write. A refusal is returned
    // as it is, so the card says why nothing changed.
    let repos = &resources.common.repos;
    let user_uuid = acting.id();
    let rejected_by = acting.caller().name();
    let Some(StatusTransition {
        user: updated_user,
        withdrawn,
        ..
    }) = admin_ops::decide_sign_up(repos, acting, SignUpDecision::Reject).await?
    else {
        return Ok(TapOutcome::LostAtTheWrite);
    };
    // The room is the authorization, so the trail row names no admin account:
    // the Slack source says which door, the log line below says which handle.
    acting
        .caller()
        .record_withdrawn(
            repos,
            &withdrawn,
            AdminStandingLost::Deactivated,
            Some(user_uuid),
        )
        .await;

    info!(
        user_id = %user_uuid,
        email = %mask_email(&updated_user.email),
        rejected_by,
        "User rejected (suspended) via Slack action"
    );

    cache_user_email(&user_uuid.to_string(), &updated_user.email);
    info!(
        target: "notify",
        event = "user.suspended",
        user_id = %user_uuid,
        suspended_by = rejected_by,
        "user account suspended"
    );

    Ok(TapOutcome::Decided(updated_user.email))
}

#[cfg(test)]
mod tests {
    use pierre_core::errors::ErrorCode;
    use pierre_core::permissions::UserRole;

    use super::*;

    fn account(role: UserRole, status: UserStatus) -> User {
        let mut user = User::new("card@example.com".to_owned(), "hash".to_owned(), None);
        user.role = role;
        user.user_status = status;
        user
    }

    /// An admin is refused whatever its status, and a sign-up never is.
    #[test]
    fn a_tap_is_refused_on_an_admin_and_on_no_one_else() {
        for status in [
            UserStatus::Active,
            UserStatus::Suspended,
            UserStatus::Pending,
        ] {
            let refused = refuse_an_admin(&account(UserRole::Admin, status))
                .expect_err("an admin is never a sign-up");
            assert_eq!(refused.code, ErrorCode::InvalidInput, "{status:?}");
            assert!(refuse_an_admin(&account(UserRole::User, status)).is_ok());
        }
    }

    /// A card decides a pending sign-up and nothing else: an account that
    /// was approved or suspended since is refused, and a pending one is not.
    #[test]
    fn a_tap_is_refused_on_an_account_that_is_no_longer_pending() {
        for status in [UserStatus::Active, UserStatus::Suspended] {
            let refused = refuse_unless_pending(&account(UserRole::User, status))
                .expect_err("only a pending account is a sign-up");
            assert_eq!(refused.code, ErrorCode::InvalidInput, "{status:?}");
        }
        assert!(refuse_unless_pending(&account(UserRole::User, UserStatus::Pending)).is_ok());
    }

    /// A tap that finds the account where its button would put it is the
    /// same decision arriving twice (carnet#1589): Approve on an active
    /// account, Reject on a suspended one. It is not refused and nothing is
    /// drawn, so the card keeps the name of whoever decided. That holds
    /// whatever the role: such a tap changes nothing on an admin either.
    #[test]
    fn a_tap_that_matches_the_accounts_standing_leaves_the_card_alone() {
        for role in [UserRole::User, UserRole::Admin] {
            for (action_type, status) in [
                (ActionType::Approve, UserStatus::Active),
                (ActionType::Reject, UserStatus::Suspended),
            ] {
                let reading = read_tap(&account(role, status), decision_of(action_type));
                assert_eq!(
                    reading.ok(),
                    Some(TapReading::AlreadyStands),
                    "{action_type:?}, account {status}, role {role:?}"
                );
            }
        }
        for action_type in [ActionType::Approve, ActionType::Reject] {
            assert_eq!(
                outcome_text(action_type, &Ok(TapOutcome::AlreadyStands), "@reviewer"),
                None,
                "{action_type:?}: the card of the tap that decided is not redrawn"
            );
        }
    }

    /// A pending sign-up is read as one to decide, on either button, and an
    /// admin is refused wherever the tap would change its standing.
    #[test]
    fn a_tap_reads_a_pending_sign_up_as_one_to_decide_and_refuses_an_admin() {
        for action_type in [ActionType::Approve, ActionType::Reject] {
            let sign_up = account(UserRole::User, UserStatus::Pending);
            assert_eq!(
                read_tap(&sign_up, decision_of(action_type)).ok(),
                Some(TapReading::Pending)
            );
            let pending_admin = account(UserRole::Admin, UserStatus::Pending);
            let refused = read_tap(&pending_admin, decision_of(action_type))
                .expect_err("an admin is never a sign-up");
            assert!(refused.to_string().contains("is an admin"), "{refused}");
        }
        for (action_type, status) in [
            (ActionType::Reject, UserStatus::Active),
            (ActionType::Approve, UserStatus::Suspended),
        ] {
            let refused = read_tap(&account(UserRole::Admin, status), decision_of(action_type))
                .expect_err("the room changes no admin's standing");
            assert!(refused.to_string().contains("is an admin"), "{refused}");
        }
    }

    /// The room reads that the account is no longer pending and what its
    /// standing is, when the tap would have changed it, and no decision is
    /// attributed.
    #[test]
    fn the_card_of_a_tap_on_a_decided_account_says_its_standing() {
        for (action_type, status, standing) in [
            (ActionType::Reject, UserStatus::Active, "it is active"),
            (
                ActionType::Approve,
                UserStatus::Suspended,
                "it is suspended",
            ),
        ] {
            let refused = read_tap(&account(UserRole::User, status), decision_of(action_type))
                .map(|_| TapOutcome::AlreadyStands);
            let card = outcome_text(action_type, &refused, "@reviewer").expect("a card is drawn");
            assert!(card.starts_with(":x: Action failed"), "{card}");
            assert!(card.contains("no longer pending"), "{card}");
            assert!(card.contains(standing), "{card}");
            assert!(card.contains("Nothing was changed"), "{card}");
            assert!(
                !card.contains("@reviewer"),
                "no decision to attribute: {card}"
            );
        }
    }

    /// The room reads why nothing was done, on either button: the Reject of
    /// an active admin's card, the Approve of a suspended one's.
    #[test]
    fn the_card_of_a_refused_tap_says_why() {
        for (action_type, status) in [
            (ActionType::Reject, UserStatus::Active),
            (ActionType::Approve, UserStatus::Suspended),
        ] {
            let refused = read_tap(&account(UserRole::Admin, status), decision_of(action_type))
                .map(|_| TapOutcome::AlreadyStands);
            let card = outcome_text(action_type, &refused, "@reviewer").expect("a card is drawn");
            assert!(card.starts_with(":x: Action failed"), "{card}");
            assert!(card.contains("is an admin"), "{card}");
            assert!(card.contains("Nothing was changed"), "{card}");
            assert!(
                !card.contains("@reviewer"),
                "no decision to attribute: {card}"
            );
        }
        assert_eq!(
            outcome_text(
                ActionType::Reject,
                &Ok(TapOutcome::Decided("new@example.com".to_owned())),
                "@reviewer"
            )
            .as_deref(),
            Some(":white_check_mark: *new@example.com* rejected (suspended) by @reviewer")
        );
    }

    /// A tap that lost at the write (carnet#1589) draws no card: there is
    /// no refusal to draw over the card of whatever decided. What the write
    /// itself does with a sign-up that is no longer pending is the
    /// service's, and is pinned there (`sign_up_decision_test`).
    #[test]
    fn a_tap_that_lost_at_the_write_draws_no_card() {
        for action_type in [ActionType::Approve, ActionType::Reject] {
            assert_eq!(
                outcome_text(action_type, &Ok(TapOutcome::LostAtTheWrite), "@reviewer"),
                None,
                "{action_type:?}: the tap that lost at the write draws no card"
            );
        }
    }

    /// An approval that was put back replaces no card either: the notice
    /// and its buttons stay for the tap that finishes, and the line under
    /// them says the sign-up is pending again, who tapped and what failed.
    #[test]
    fn an_approval_that_was_put_back_keeps_the_card_and_says_so_under_it() {
        let put_back = Ok(TapOutcome::PutBack("Database error".to_owned()));
        assert_eq!(
            outcome_text(ActionType::Approve, &put_back, "@reviewer"),
            None,
            "the card keeps its notice and its buttons"
        );
        let line = put_back_line("@reviewer", "Database error");
        assert!(line.starts_with("@reviewer: "), "{line}");
        assert!(line.contains("did not complete (Database error)"), "{line}");
        assert!(line.contains("pending again"), "{line}");
        assert!(line.contains("from the console"), "{line}");
    }
}
