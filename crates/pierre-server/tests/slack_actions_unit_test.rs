// ABOUTME: Tests for slack_actions authorization: ops-channel trust and command-postback gating
// ABOUTME: Covers channel_matches plus the channel-link / user-status gate on card-button postbacks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slack action authorization tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate, so without a surviving crate doc the command-line
// `-D warnings` trips `missing_docs`.
#![cfg(feature = "client-messaging")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use pierre_mcp_server::routes::messaging::slack_actions::channel_matches;

#[test]
fn matches_configured_channel_by_name_with_hash() {
    // Config carries the `#` prefix (as set in terraform); payload name omits it.
    assert!(channel_matches(
        "dravr-dev-users",
        "C0ABC123",
        "#dravr-dev-users"
    ));
}

#[test]
fn matches_configured_channel_by_name_without_hash() {
    assert!(channel_matches(
        "dravr-dev-users",
        "C0ABC123",
        "dravr-dev-users"
    ));
}

#[test]
fn matches_configured_channel_by_id() {
    // Operators may configure a raw channel ID instead of a name.
    assert!(channel_matches("", "C0ABC123", "C0ABC123"));
}

#[test]
fn tolerates_surrounding_whitespace_in_config() {
    assert!(channel_matches(
        "dravr-dev-users",
        "C0ABC123",
        "  #dravr-dev-users  "
    ));
}

#[test]
fn rejects_other_channel() {
    assert!(!channel_matches(
        "random-chat",
        "C0XYZ999",
        "#dravr-dev-users"
    ));
}

#[test]
fn rejects_dm_channel() {
    // DMs report a name like "directmessage" and an ID starting with `D`.
    assert!(!channel_matches(
        "directmessage",
        "D0ABC123",
        "#dravr-dev-users"
    ));
}

#[test]
fn rejects_empty_configured_channel_fails_closed() {
    // An unset/blank channel must never authorize, even against a blank payload.
    assert!(!channel_matches("dravr-dev-users", "C0ABC123", ""));
    assert!(!channel_matches("dravr-dev-users", "C0ABC123", "   "));
    assert!(!channel_matches("", "", ""));
}

/// Command postbacks (card buttons whose `action_id` is a `/command`) must
/// clear the same gate every other inbound messaging turn clears: a
/// `messaging_channel_links` row for the Slack identity, then the approval
/// policy in `enforce_user_status`. The HMAC signature proves the click came
/// from Slack; it says nothing about whether that Slack identity may act as a
/// Pierre user.
mod command_postback_authorization {
    use crate::common::create_test_server_resources;
    use chrono::Utc;
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::slack_actions::execute_postback_command;
    use uuid::Uuid;

    /// Slack DM channel the card button lives in (`D` prefix = IM).
    const CARD_CHANNEL: &str = "D0CARDTEST";

    /// Create a user with the requested status plus a tenant they own.
    ///
    /// The password hash is a literal: nothing in the postback path
    /// authenticates a password, so a real bcrypt round would only slow the
    /// test down.
    async fn create_user(
        resources: &ServerContext,
        email: &str,
        status: UserStatus,
    ) -> (Uuid, TenantId) {
        let mut user = User::new(
            email.to_owned(),
            "not-a-login-path".to_owned(),
            Some("Postback User".to_owned()),
        );
        user.user_status = status;
        if status == UserStatus::Active {
            user.approved_by = Some(user.id);
            user.approved_at = Some(Utc::now());
        }
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Postback Tenant {email}"),
            slug: format!("postback-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();

        (user_id, tenant_id)
    }

    /// Link a Slack identity to a Pierre user — the row the OTP / link-code
    /// flow writes and the only proof the clicker owns the account.
    async fn link_slack(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        user_id: Uuid,
        slack_user_id: &str,
    ) {
        let link_id = Uuid::new_v4().to_string();
        let user_id_str = user_id.to_string();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id_str,
            channel_type: "slack",
            channel_user_id: slack_user_id,
            display_name: Some("Slack Clicker"),
        })
        .await
        .unwrap();
    }

    async fn analytics_consent(resources: &ServerContext, user_id: Uuid) -> bool {
        resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap()
            .analytics_consent
    }

    /// The legitimate flow: a linked, Active user clicking a card button gets
    /// the command executed. Asserts the side effect (`analytics_consent`
    /// flipped in the database), not just a returned `Ok`.
    #[tokio::test]
    async fn linked_active_user_postback_executes_the_command() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) =
            create_user(&resources, "slack_active@example.com", UserStatus::Active).await;
        link_slack(db, tenant_id, user_id, "U0ACTIVE01").await;

        assert!(
            !analytics_consent(&resources, user_id).await,
            "consent starts disabled"
        );

        let text = execute_postback_command(&resources, "U0ACTIVE01", CARD_CHANNEL, "/privacy on")
            .await
            .expect("a linked Active user must still be able to click a card button");

        assert!(
            !text.trim().is_empty(),
            "postback reply text must be populated"
        );
        assert!(
            analytics_consent(&resources, user_id).await,
            "the command must actually have run for the linked Active user"
        );
    }

    /// Pending accounts are denied on every other transport by
    /// `enforce_user_status`; a Slack card button is not an exception.
    #[tokio::test]
    async fn pending_user_postback_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) =
            create_user(&resources, "slack_pending@example.com", UserStatus::Pending).await;
        link_slack(db, tenant_id, user_id, "U0PENDING1").await;

        let err = execute_postback_command(&resources, "U0PENDING1", CARD_CHANNEL, "/privacy on")
            .await
            .expect_err("a Pending user must not execute commands from a card button");

        assert_eq!(err.code, ErrorCode::AccountPending);
        assert!(
            !analytics_consent(&resources, user_id).await,
            "a refused postback must not have run the command"
        );
    }

    /// Symmetric to the Pending case: a suspended account keeps its buttons
    /// but not its authority.
    #[tokio::test]
    async fn suspended_user_postback_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) = create_user(
            &resources,
            "slack_suspended@example.com",
            UserStatus::Suspended,
        )
        .await;
        link_slack(db, tenant_id, user_id, "U0SUSPEND1").await;

        let err = execute_postback_command(&resources, "U0SUSPEND1", CARD_CHANNEL, "/privacy on")
            .await
            .expect_err("a Suspended user must not execute commands from a card button");

        assert_eq!(err.code, ErrorCode::AccountSuspended);
        assert!(
            !analytics_consent(&resources, user_id).await,
            "a refused postback must not have run the command"
        );
    }

    /// Identity rests on the channel link, not on the profile email Slack
    /// reports for the clicker. An Active Pierre user who never linked their
    /// Slack identity cannot be acted for, however plausible the email.
    #[tokio::test]
    async fn unlinked_slack_identity_postback_is_refused() {
        let resources = create_test_server_resources().await.unwrap();

        let (user_id, _tenant_id) =
            create_user(&resources, "slack_unlinked@example.com", UserStatus::Active).await;

        let err = execute_postback_command(&resources, "U0UNLINKED", CARD_CHANNEL, "/privacy on")
            .await
            .expect_err("an unlinked Slack identity must not execute commands");

        assert_eq!(err.code, ErrorCode::AuthInvalid);
        assert!(
            !analytics_consent(&resources, user_id).await,
            "a refused postback must not have run the command"
        );
    }
}
