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

use pierre_core::errors::ErrorCode;
use pierre_mcp_server::routes::messaging::slack_actions::{channel_matches, parse_slack_action};
use serde_json::json;

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
    // Admins may configure a raw channel ID instead of a name.
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

/// Form-encode a Slack interactive payload the way Slack's client does:
/// `application/x-www-form-urlencoded`, so a space travels as `+` and a
/// literal `+` as `%2B`.
fn interactive_body(action_id: &str, username: &str) -> Vec<u8> {
    let payload = json!({
        "type": "block_actions",
        "user": { "id": "U0CLICKER", "username": username },
        "channel": { "id": "D0CARDTEST", "name": "directmessage" },
        "message": { "ts": "1727440000.000100" },
        "actions": [{ "action_id": action_id }],
    })
    .to_string();
    format!(
        "payload={}",
        urlencoding::encode(&payload).replace("%20", "+")
    )
    .into_bytes()
}

#[test]
fn an_encoded_literal_plus_in_a_postback_stays_a_plus() {
    let body = interactive_body("/note 2+2 is four", "jane");
    assert!(
        String::from_utf8_lossy(&body).contains("2%2B2+is+four"),
        "the fixture must carry both encodings"
    );

    let action = parse_slack_action(&body).unwrap();

    assert_eq!(action.action_id, "/note 2+2 is four");
}

#[test]
fn a_plus_encoded_space_decodes_to_a_space_in_every_field() {
    let body = interactive_body("/agent add coach", "Jane Doe");
    assert!(String::from_utf8_lossy(&body).contains("Jane+Doe"));

    let action = parse_slack_action(&body).unwrap();

    assert_eq!(action.action_id, "/agent add coach");
    assert_eq!(action.slack_username, "Jane Doe");
    assert_eq!(action.slack_user_id, "U0CLICKER");
    assert_eq!(action.channel_id, "D0CARDTEST");
    assert_eq!(action.channel_name, "directmessage");
    assert_eq!(action.message_ts, "1727440000.000100");
}

#[test]
fn a_body_without_a_payload_field_is_refused() {
    let err = parse_slack_action(b"token=abc&team_id=T0").unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

/// Command postbacks (card buttons whose `action_id` is a `/command`) must
/// clear the same gate every other inbound messaging turn clears: a
/// `messaging_channel_links` row for the Slack identity, then the approval
/// policy in `enforce_user_status`. The HMAC signature proves the click came
/// from Slack; it says nothing about whether that Slack identity may act as a
/// Pierre user.
mod command_postback_authorization {
    use crate::common::create_test_server_resources;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
    use pierre_database::repositories::{
        CreateSessionParams, PendingAgentDraft, AGENT_PROPOSAL_ACTION,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::slack_actions::execute_postback_command;
    use serde_json::json;
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
            user.approved_at = Some(Timestamp::now());
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
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
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

    /// A draft `/agent create` parked for `user_id`, drafted in
    /// `conversation_id`: its claim token.
    async fn park_draft(
        resources: &ServerContext,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
    ) -> String {
        let token = Uuid::new_v4().simple().to_string();
        resources
            .common
            .repos
            .agent_drafts
            .create_pending_action(
                &PendingAgentDraft {
                    id: token.clone(),
                    tenant_id: tenant_id.to_string(),
                    user_id: user_id.to_string(),
                    tool_name: AGENT_PROPOSAL_ACTION.to_owned(),
                    arguments: json!({
                        "title": "Hill Agent",
                        "description": "Coaches hill repeats.",
                        "system_prompt": "You coach hill repeats.",
                        "category": "training",
                        "tags": [],
                        "conversation_id": conversation_id,
                    }),
                    deny_reason: "awaiting_confirmation".to_owned(),
                },
                Timestamp::now() + Elapsed::minutes(10),
            )
            .await
            .unwrap();
        token
    }

    /// carnet#1511 design call 1: a draft is confirmed only from the thread
    /// it was drafted in. The "Create it" button on a Slack DM's card acts in
    /// the thread the DM's session writes into, so it confirms the draft
    /// drafted there and no other.
    #[tokio::test]
    async fn a_dm_cards_confirm_button_acts_in_the_dms_thread() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = &resources.common.repos;
        let db: &dyn MessagingRepository = &*repos.messaging;
        let (user_id, tenant_id) =
            create_user(&resources, "slack_draft@example.com", UserStatus::Active).await;
        link_slack(db, tenant_id, user_id, "U0DRAFTER1").await;
        let thread = repos
            .chat
            .create_conversation(&user_id.to_string(), tenant_id, "DM", "model", None, None)
            .await
            .unwrap()
            .id;
        db.create_session(&CreateSessionParams {
            id: &Uuid::new_v4().to_string(),
            user_id: &user_id.to_string(),
            tenant_id,
            channel_type: "slack",
            channel_user_id: "U0DRAFTER1",
            channel_conversation_id: Some(CARD_CHANNEL),
            pierre_conversation_id: Some(&thread),
        })
        .await
        .unwrap();

        let elsewhere = park_draft(&resources, user_id, tenant_id, "another-thread").await;
        let refused = execute_postback_command(
            &resources,
            "U0DRAFTER1",
            CARD_CHANNEL,
            &format!("/agent create confirm {elsewhere}"),
        )
        .await
        .unwrap();
        assert!(
            !refused.contains("Hill Agent"),
            "a draft from another thread is not confirmed here: {refused}"
        );

        let here = park_draft(&resources, user_id, tenant_id, &thread).await;
        let created = execute_postback_command(
            &resources,
            "U0DRAFTER1",
            CARD_CHANNEL,
            &format!("/agent create confirm {here}"),
        )
        .await
        .unwrap();
        assert!(
            created.contains("Hill Agent"),
            "the DM's card confirms the draft drafted in its thread: {created}"
        );
    }

    /// Slack channel the room card's button lives in (`C` prefix = channel).
    const ROOM_CHANNEL: &str = "C0ROOMCARD";

    /// carnet#1511 design call 1, a channel: the "Create it" button on a
    /// card posted in a Slack channel acts in the thread that channel's
    /// session writes into for the clicker, found as an inbound channel
    /// message finds it: under the tenant the clicker's channel link lives
    /// in, here a workspace bot's tenant that is not the clicker's own. A
    /// channel with no session for the clicker names no thread, and the
    /// button says what to type in the draft's own conversation.
    #[tokio::test]
    async fn a_channel_cards_confirm_button_acts_in_the_channels_thread() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = &resources.common.repos;
        let db: &dyn MessagingRepository = &*repos.messaging;
        let (user_id, tenant_id) =
            create_user(&resources, "slack_room@example.com", UserStatus::Active).await;
        let (_, bot_tenant) = create_user(
            &resources,
            "slack_bot_owner@example.com",
            UserStatus::Active,
        )
        .await;
        link_slack(db, bot_tenant, user_id, "U0ROOMDRAFT").await;
        let thread = repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                bot_tenant,
                "Room",
                "model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let confirm = |token: &str| format!("/agent create confirm {token}");

        // No session for the clicker in this channel yet: no thread named.
        let waiting = park_draft(&resources, user_id, tenant_id, &thread).await;
        let told =
            execute_postback_command(&resources, "U0ROOMDRAFT", ROOM_CHANNEL, &confirm(&waiting))
                .await
                .unwrap();
        assert!(
            told.contains(&confirm(&waiting)) && !told.contains("Hill Agent"),
            "a channel with no session says what to type, and confirms nothing: {told}"
        );

        db.create_session(&CreateSessionParams {
            id: &Uuid::new_v4().to_string(),
            user_id: &user_id.to_string(),
            tenant_id: bot_tenant,
            channel_type: "slack",
            channel_user_id: "U0ROOMDRAFT",
            channel_conversation_id: Some(ROOM_CHANNEL),
            pierre_conversation_id: Some(&thread),
        })
        .await
        .unwrap();
        let created =
            execute_postback_command(&resources, "U0ROOMDRAFT", ROOM_CHANNEL, &confirm(&waiting))
                .await
                .unwrap();
        assert!(
            created.contains("Hill Agent"),
            "the channel's card confirms the draft drafted in its thread: {created}"
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

/// Property tests: an interactive payload from the internet is parsed or
/// refused, never a panic (carnet#1143).
mod parse_properties {
    use proptest::collection::vec;
    use proptest::prelude::*;

    use pierre_mcp_server::routes::messaging::slack_actions::parse_slack_action;

    proptest! {
        #[test]
        fn any_body_is_parsed_without_panicking(body in vec(any::<u8>(), 0..512)) {
            // Only checks that parsing does not panic.
            drop(parse_slack_action(&body));
        }

        #[test]
        fn any_form_encoded_payload_is_parsed_without_panicking(payload in any::<String>()) {
            let body = format!("payload={}", urlencoding::encode(&payload));
            // Only checks that parsing does not panic.
            drop(parse_slack_action(body.as_bytes()));
        }

        #[test]
        fn a_block_actions_shape_with_odd_fields_is_parsed_without_panicking(
            action_id in prop_oneof![Just(String::new()), any::<String>()],
            user in any::<String>(),
            channel in any::<String>(),
            ts in any::<String>(),
        ) {
            let payload = serde_json::json!({
                "type": "block_actions",
                "actions": [{ "action_id": action_id, "value": user }],
                "user": { "id": user },
                "channel": { "id": channel },
                "message": { "ts": ts },
                "container": { "message_ts": ts },
            });
            let body = format!("payload={}", urlencoding::encode(&payload.to_string()));
            // Only checks that parsing does not panic.
            drop(parse_slack_action(body.as_bytes()));
        }
    }
}
