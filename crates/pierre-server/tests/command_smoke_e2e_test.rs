// ABOUTME: Catalog-driven e2e smoke — every slash command answers content over the real wire, DM and room
// ABOUTME: Two lanes: DM (ledgered reply, zero LLM) and room (delivery matches the room-visibility contract)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The framework half of the command e2e suite: both lanes iterate the LOADED
//! production catalog, so a new `commands/*.md` file joins the matrix by
//! existing — there is no per-command code and no exception list to maintain.
//!
//! What each lane adds over `messaging_command_coverage_test` (recognition on
//! five channels) is CONTENT: the reply must actually reach the outbound
//! ledger — the record of what the athlete was told — and must never fall
//! through to the LLM, and in a room its delivery must match the
//! room-visibility contract the ingress enforces.
//!
//! Two load-bearing preconditions, uniform across the matrix:
//!
//! - **A fresh member (and room) per command.** Commands mutate the very
//!   state the lane observes — `/logout` severs the link, `/calibrate`
//!   leaves a live walk — so sharing a member would make the matrix
//!   order-dependent.
//! - **A `/status` prime before the command under test.** The prime forges
//!   the member's session, which is where the ledger attaches replies; it
//!   also keeps the lane honest for `/logout`, whose reply is answered before
//!   the slash dispatcher and is ledgered against the already-existing
//!   session, and for `/reset`, which rotates that session onto a fresh
//!   conversation. The prime's own reply is the per-member baseline row.
//! - **A fresh conversation per command** (implied by the fresh member): a
//!   command that inspects the conversation — `/coach create` reads its
//!   coaching turns — must meet the empty conversation it meets in
//!   production right after linking, or it reaches for the LLM.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod command_smoke {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{commands_dir, CommandE2e, RoomE2e, RouterLlm};
    use pierre_commands::load_command_catalog;
    use pierre_core::models::groups::GroupRespondMode;
    use pierre_mcp_server::services::messaging_ingress::slash_reply_should_be_private;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    const ROOM_CHAT_BASE: i64 = -100_822_000;

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn every_command_answers_ledgered_content_in_a_dm_without_an_llm() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["OK."]);

        let definitions = load_command_catalog(&commands_dir()).definitions;
        assert!(
            !definitions.is_empty(),
            "no command definitions loaded — the matrix would pass vacuously"
        );

        let mut checked = 0_usize;
        for def in &definitions {
            let member = e2e.linked_member(false).await;

            // Prime: forge the session and the baseline ledger row.
            let prime = e2e.send_dm(&member, "/status").await;
            assert_eq!(
                prime.messages_stored(),
                0,
                "the /status prime must dispatch as a command"
            );
            let session = e2e
                .session_id(&member, member.home_tenant, &member.channel_user_id)
                .await
                .expect("the prime must forge a DM session");
            let baseline = e2e.wait_outbound_for_session(&session, 1).await;

            let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);
            let ack = e2e.send_dm(&member, &def.command).await;
            assert_eq!(
                ack.messages_stored(),
                0,
                "`{}` was not recognized as a command in a DM",
                def.command
            );

            // The reply is real, non-empty, and on the ledger.
            e2e.wait_outbound_for_session(&session, baseline + 1).await;

            // Slash replies are deterministic: a command that fell through to
            // the LLM is a routing regression, not a slower answer.
            assert_eq!(
                e2e.llm.turn_calls.load(Ordering::SeqCst),
                turns_before,
                "`{}` must not reach the LLM in a DM",
                def.command
            );
            checked += 1;
        }
        assert_eq!(
            checked,
            definitions.len(),
            "every definition must be checked"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn every_command_honours_the_room_visibility_contract() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["OK."]);

        let definitions = load_command_catalog(&commands_dir()).definitions;
        assert!(
            !definitions.is_empty(),
            "no command definitions loaded — the matrix would pass vacuously"
        );

        let mut checked = 0_usize;
        for (i, def) in definitions.iter().enumerate() {
            let member = e2e.linked_member(false).await;
            let chat_id = ROOM_CHAT_BASE - i64::try_from(i).unwrap();
            let room =
                RoomE2e::bind_room(Arc::clone(&e2e), chat_id, GroupRespondMode::All, &member).await;

            // Prime: forge the room session (the prime's reply is private —
            // /status is personal — so it adds no room-visible chat rows).
            let prime = room.send_room_slash(&member, "/status").await;
            assert_eq!(prime.messages_stored(), 0);
            let session = room
                .session_id(&member)
                .await
                .expect("the prime must forge the room session");
            let baseline = e2e.wait_outbound_for_session(&session, 1).await;
            let conversation = room
                .conversation_id(&member)
                .await
                .expect("the room session names its conversation");

            let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);
            let ack = room.send_room_slash(&member, &def.command).await;
            assert_eq!(
                ack.messages_stored(),
                0,
                "`{}` was not recognized as a command in a room",
                def.command
            );
            e2e.wait_outbound_for_session(&session, baseline + 1).await;
            assert_eq!(
                e2e.llm.turn_calls.load(Ordering::SeqCst),
                turns_before,
                "`{}` must not reach the LLM in a room",
                def.command
            );

            // Delivery matches the ingress's own contract, derived from the
            // SAME source production reads (`slash_reply_should_be_private`)
            // — never from the `personal:` menu marker, which is a different
            // axis. Room-visible commands persist their turn into the room
            // conversation; private ones leave no trace of the command there.
            let rows = room.chat_rows_carrying(&conversation, &def.command).await;
            if slash_reply_should_be_private(false, Some(&def.name)) {
                assert_eq!(
                    rows, 0,
                    "`{}` is private in a room — its turn must not be persisted there",
                    def.command
                );
            } else {
                assert!(
                    rows > 0,
                    "`{}` is room-visible — its turn must be persisted in the room",
                    def.command
                );
            }
            checked += 1;
        }
        assert_eq!(
            checked,
            definitions.len(),
            "every definition must be checked"
        );
    }
}
