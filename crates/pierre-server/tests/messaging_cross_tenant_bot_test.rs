// ABOUTME: Regression test — an admin-owned bot whose tenant differs from the user's own tenant must
// ABOUTME: still drive a full DM turn: session/conversation/messages land under the USER tenant, LLM runs.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

// This guards the precise bug the messaging-session user-tenant fix targets: a
// Telegram bot owned by tenant B (the webhook/channel tenant) serving a user
// whose active_tenant_id is their OWN tenant A. The session + conversation +
// messages must be stored under tenant A (so they align with the user's activity
// cache and the backfill push), while the channel link + config stay under
// tenant B. Before the fix, the conversation was created under tenant A but the
// live dispatch read it under tenant B → get_conversation missed → every turn
// failed with "Conversation not found" before the LLM ever ran. The same-tenant
// case (channel == user) masks this, so only a cross-tenant fixture catches it.
//
// The harness builds exactly that topology: every `linked_member` owns their
// own tenant and is channel-linked under the bot tenant.
#[cfg(feature = "client-messaging")]
mod cross_tenant_bot_tests {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, RouterLlm};
    use chrono::Utc;
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
    use serial_test::serial;
    use std::env;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// A supergroup no coaching group is bound to before the first message.
    const UNBOUND_CHAT_ID: i64 = -1_001_234;

    async fn start() -> Arc<CommandE2e> {
        // chat_conversations creation reads PIERRE_LLM_MODEL; the value is stored
        // on the row, never dispatched (the mock answers the actual call).
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        CommandE2e::start(resources, llm).await
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn cross_tenant_bot_dm_stores_under_user_tenant_and_reaches_llm() {
        let e2e = start().await;
        e2e.llm.set_turn_replies(&["Voici tes courses."]);

        // The user and THEIR own tenant (active_tenant_id resolves here), with a
        // synthetic provider so the onboarding gate lets the turn reach the LLM.
        // The user is NOT a member of the bot tenant, so it is purely the
        // channel/webhook tenant — exactly the admin-owned-bot shape that
        // fragmented jf's data.
        let user = e2e.linked_member(true).await;
        assert_ne!(
            user.home_tenant, e2e.bot_tenant,
            "fixture must be cross-tenant"
        );

        // Inbound Telegram DM: chat.id == from.id makes it a direct message, so
        // session_tenant resolves to the user's own tenant.
        e2e.send_dm(&user, "Donne moi mes courses en 2022").await;

        // The pipeline must reach the LLM — proof the conversation read under the
        // user tenant succeeded (the bug timed out here at "Conversation not found").
        assert!(
            e2e.wait_llm_turns(1).await,
            "pipeline never reached the LLM — conversation read likely missed under the wrong tenant"
        );

        // Assert the whole DM unit lives under the USER tenant, not the bot tenant.
        let session = e2e
            .session_id(&user, user.home_tenant, &user.channel_user_id)
            .await
            .expect("session must live under the user's own tenant, not the bot tenant");
        assert!(
            e2e.session_id(&user, e2e.bot_tenant, &user.channel_user_id)
                .await
                .is_none(),
            "no session may be filed under the bot tenant"
        );

        let conversation_id = e2e
            .conversation_id(&user, user.home_tenant, &user.channel_user_id)
            .await
            .expect("the session names its conversation");
        assert!(
            e2e.resources
                .common
                .repos
                .chat
                .get_conversation(
                    &conversation_id,
                    &user.user_id.to_string(),
                    user.home_tenant
                )
                .await
                .unwrap()
                .is_some(),
            "conversation must live under the user tenant so the pipeline can read it"
        );

        // The inbound message row must share the user tenant (no re-split).
        let user_tenant = user.home_tenant.to_string();
        assert_eq!(
            e2e.ledger_tenants_for_session(&session, "inbound").await,
            vec![user_tenant.clone()],
            "the inbound message must be stored under the user tenant and only there"
        );

        // The outbound (assistant) row must also land under the user tenant —
        // the other half of the DM unit. The fake bot token makes the real
        // Telegram send fail, so the reply persists via the retry path
        // (outbound_retry::enqueue_failed_outbound), which likewise uses
        // session_tenant_id for the message row; it is written asynchronously
        // after the failed delivery.
        e2e.wait_outbound_for_session(&session, 1).await;
        assert_eq!(
            e2e.ledger_tenants_for_session(&session, "outbound").await,
            vec![user_tenant],
            "outbound message must be stored under the user tenant, not the bot tenant"
        );
    }

    /// The same cross-tenant topology, but for a GROUP chat and a slash command.
    ///
    /// A group session, its conversation and the `coaching_groups` row auto-bound
    /// to the chat all live under the BOT tenant, while the member's
    /// `active_tenant_id` is their own. `/group consent yes` is a privacy control:
    /// it must flip `peer_sharing_consent` on the group that owns the chat it was
    /// typed in. When the slash dispatcher looked the conversation up under the
    /// caller's tenant instead, the read missed and the handler fell through to
    /// `list_groups_for_user().first()` — an unfiltered, cross-tenant,
    /// `updated_at DESC` list — and published the athlete's training data to the
    /// members of whichever OTHER group they had touched most recently.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn cross_tenant_group_slash_consent_binds_to_the_chat_group() {
        let e2e = start().await;
        e2e.llm
            .set_turn_replies(&["unused — slash commands skip the LLM"]);

        let user = e2e.linked_member(true).await;
        assert_ne!(
            user.home_tenant, e2e.bot_tenant,
            "fixture must be cross-tenant"
        );

        // The channel-group bootstrap picks a system agent from the BOT tenant;
        // without one it declines to create the group and the chat stays
        // ungrouped.
        e2e.seed_coach_agent(e2e.bot_owner, e2e.bot_tenant, "Group Slash Coach")
            .await;

        // A second group in the member's OWN tenant, stamped in the future so it
        // heads `list_groups_for_user` (ORDER BY updated_at DESC). This is the
        // row consent must NOT land on.
        let decoy_coach = e2e
            .seed_coach_agent(user.user_id, user.home_tenant, "Decoy Coach")
            .await;
        let decoy_group_id = Uuid::new_v4();
        let now = Utc::now();
        e2e.resources
            .common
            .repos
            .groups
            .create_group(
                user.home_tenant,
                &CoachingGroup {
                    id: decoy_group_id,
                    tenant_id: user.home_tenant.to_string(),
                    name: "Decoy group".to_owned(),
                    description: None,
                    agent_id: decoy_coach.id.to_string(),
                    owner_id: user.user_id,
                    coach_user_id: None,
                    peer_data_sharing: true,
                    respond_mode: GroupRespondMode::default(),
                    max_members: 10,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now + chrono::Duration::seconds(600),
                },
            )
            .await
            .unwrap();
        e2e.resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: decoy_group_id,
                user_id: user.user_id,
                tenant_id: user.home_tenant.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();

        // chat.id != from.id and chat.type = "supergroup" → not a DM, so the
        // session, conversation and coaching group land under the bot tenant.
        let sender: i64 = user.channel_user_id.parse().unwrap();
        e2e.send_group_text(sender, UNBOUND_CHAT_ID, "/group consent yes")
            .await;

        // The chat's coaching group is created during session resolution; poll
        // until it exists, then until the consent write lands.
        let groups = &e2e.resources.common.repos.groups;
        let mut chat_group = None;
        for _ in 0..50 {
            chat_group = groups
                .get_group_by_channel(e2e.bot_tenant, "telegram", &UNBOUND_CHAT_ID.to_string())
                .await
                .unwrap();
            if chat_group.is_some() {
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        let chat_group =
            chat_group.expect("the supergroup chat must auto-create a coaching_groups row");
        assert_eq!(
            chat_group.tenant_id,
            e2e.bot_tenant.to_string(),
            "a shared room's coaching group belongs to the channel tenant"
        );

        let mut chat_consent = false;
        for _ in 0..50 {
            chat_consent = groups
                .list_members(&chat_group.id.to_string())
                .await
                .unwrap()
                .iter()
                .find(|m| m.user_id == user.user_id)
                .is_some_and(|m| m.peer_sharing_consent);
            if chat_consent {
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        assert!(
            chat_consent,
            "/group consent yes must flip consent on the group bound to the chat \
             it was typed in, even though the member's tenant is not the bot's"
        );

        let decoy_consent = groups
            .list_members(&decoy_group_id.to_string())
            .await
            .unwrap()
            .iter()
            .find(|m| m.user_id == user.user_id)
            .expect("decoy membership row")
            .peer_sharing_consent;
        assert!(
            !decoy_consent,
            "consent must not leak into the member's other, more recently updated group"
        );
    }
}
