// ABOUTME: Approve or reject the account a sign-up notice names, from its Slack Approve/Reject buttons
// ABOUTME: The room authorizes sign-ups and no more: an admin account is refused, and the card says why
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::StatusCode;
use axum::Json;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Tenant, User, UserStatus};
use pierre_database::repositories::AuditSource;
use pierre_middleware::mask_email;
use pierre_services::admin_ops::{
    record_withdrawn_admin_credentials, transition_user_status, AdminStandingLost, StandingChange,
    StatusTransition,
};
use pierre_services::analytics::cache_user_email;
use pierre_services::tenant_admin as tenant_admin_service;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

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
/// A rejection goes through the suspension every other surface uses. An
/// account that is not an admin can still hold what one did (a device login
/// or an impersonation left live by a build that demoted without
/// withdrawing them): the suspension withdraws those, on the trail under
/// the Slack source.
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

    let blocks = json!([{
        "type": "section",
        "text": { "type": "mrkdwn", "text": outcome_text(action_type, &result, &approver) }
    }]);
    // Fire-and-forget message update via tronc SlackClient
    update_card(resources, action, &blocks);

    // Return 200 immediately (Slack expects a response within 3 seconds)
    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

/// What the card becomes: the outcome and who tapped, or why nothing was
/// done.
fn outcome_text(action_type: ActionType, result: &AppResult<String>, approver: &str) -> String {
    match result {
        Ok(email) => match action_type {
            ActionType::Approve => format!(":white_check_mark: *{email}* approved by {approver}"),
            ActionType::Reject => {
                format!(":white_check_mark: *{email}* rejected (suspended) by {approver}")
            }
        },
        Err(e) => format!(":x: Action failed: {e}"),
    }
}

/// Take the decision a tap names on the account it names; the account's
/// email.
///
/// # Errors
///
/// Returns a not-found error for an unknown account, an invalid-input error
/// for an admin account ([`refuse_an_admin`]), and whatever the approval or
/// the rejection fails with.
async fn decide(
    resources: &ServerContext,
    action_type: ActionType,
    user_uuid: Uuid,
    tapped_by: &str,
) -> AppResult<String> {
    let user = resources
        .common
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    refuse_an_admin(&user)?;
    match action_type {
        ActionType::Approve => approve_user(resources, user, tapped_by).await,
        ActionType::Reject => reject_user(resources, user, tapped_by).await,
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

/// Approve a user: set status to Active and create tenant if needed
async fn approve_user(
    resources: &ServerContext,
    user: User,
    approved_by: &str,
) -> AppResult<String> {
    let user_uuid = user.id;
    if user.user_status == UserStatus::Active {
        return Ok(user.email);
    }

    let updated_user = resources
        .common
        .repos
        .users
        .update_status(user_uuid, UserStatus::Active, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to approve user: {e}")))?
        .user;

    // Create default tenant for the approved user if they don't have one
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_uuid)
        .await
        .unwrap_or_default();

    // The tenant user.approved is attributed to, the same one sign-up keyed
    // its events on, so the two land on one identity.
    let tenant_id = if tenants.is_empty() {
        let provisioned = tenant_admin_service::provision_tenant_for_approval(
            &resources.common.repos,
            user_uuid,
            &updated_user.email,
            updated_user.display_name.as_deref(),
            None,
            None,
        )
        .await?;
        Some(provisioned.id)
    } else {
        Tenant::attribution_tenant(&tenants, user_uuid)
    };

    info!(
        user_id = %user_uuid,
        email = %mask_email(&updated_user.email),
        approved_by,
        "User approved via Slack action"
    );

    // Warm the identity cache first: the notify enricher resolves user_email
    // from user_id, and without it the Slack message loses the subject.
    cache_user_email(&user_uuid.to_string(), &updated_user.email);
    info!(
        target: "notify",
        event = "user.approved",
        user_id = %user_uuid,
        tenant_id = tenant_id.map(display),
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

    Ok(updated_user.email)
}

/// Reject (suspend) a user
async fn reject_user(
    resources: &ServerContext,
    user: User,
    rejected_by: &str,
) -> AppResult<String> {
    let user_uuid = user.id;
    if user.user_status == UserStatus::Suspended {
        return Ok(user.email);
    }

    // The one suspension path every surface takes: what the account still
    // held from a time as an admin goes in the same write. A refusal is
    // returned as it is, so the card says why nothing changed.
    let repos = &resources.common.repos;
    let StatusTransition {
        user: updated_user,
        withdrawn,
    } = transition_user_status(repos, user_uuid, UserStatus::Suspended, None, None).await?;
    // The room is the authorization, so the trail row names no admin account:
    // the Slack source says which door, the log line below says which handle.
    record_withdrawn_admin_credentials(
        repos,
        &withdrawn,
        StandingChange {
            cause: AdminStandingLost::Deactivated,
            account: Some(user_uuid),
            actor: None,
            source: AuditSource::Slack,
        },
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

    Ok(updated_user.email)
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

    /// The room reads why nothing was done, on either button.
    #[test]
    fn the_card_of_a_refused_tap_says_why() {
        let refused =
            refuse_an_admin(&account(UserRole::Admin, UserStatus::Active)).map(|()| String::new());
        for action_type in [ActionType::Approve, ActionType::Reject] {
            let card = outcome_text(action_type, &refused, "@reviewer");
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
                &Ok("new@example.com".to_owned()),
                "@reviewer"
            ),
            ":white_check_mark: *new@example.com* rejected (suspended) by @reviewer"
        );
    }
}
