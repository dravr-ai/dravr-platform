// ABOUTME: Integration tests for the group respond-mode gate — mentions mode silences ambient
// ABOUTME: chatter (captured for the room transcript) while addressed turns reach the LLM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod respond_mode_tests {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RoomE2e, RouterLlm};
    use pierre_core::models::groups::GroupRespondMode;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    /// Telegram supergroup chat id every respond-mode room is bound to.
    const GROUP_CHAT_ID: i64 = -100_999_777;
    /// A channel sender id no member holds a link for.
    const UNLINKED_SENDER: i64 = 77;

    /// A server on the production LLM wiring, a bot tenant, and a room in
    /// `mode` owned by a linked, provider-connected member.
    async fn room_in(mode: GroupRespondMode) -> (Arc<CommandE2e>, RoomE2e, Member) {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        let owner = e2e.linked_member(true).await;
        let room = RoomE2e::bind_room(Arc::clone(&e2e), GROUP_CHAT_ID, mode, &owner).await;
        (e2e, room, owner)
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn mentions_mode_silences_ambient_and_injects_transcript_on_addressed_turn() {
        let (e2e, room, owner) = room_in(GroupRespondMode::Mentions).await;
        e2e.llm.set_turn_replies(&["On en parle."]);

        let ambient_text = "nice tempo run everyone, felt easy today";

        // 1. Unaddressed group message from the LINKED owner: no LLM turn, no
        //    reply — but the row is captured for the room transcript.
        room.send_room(&owner, ambient_text).await;
        sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            0,
            "mentions mode must not dispatch an unaddressed group message to the LLM"
        );
        assert_eq!(
            e2e.count_inbound_with_body(ambient_text).await,
            1,
            "ambient message must be stored for the room transcript"
        );

        // 2. Unaddressed message from an UNLINKED sender: silently dropped —
        //    no link prompt, no stored row, still no LLM turn.
        let unlinked_text = "I am not linked and just chatting";
        room.send_room_from(UNLINKED_SENDER, unlinked_text).await;
        sleep(Duration::from_millis(800)).await;
        assert_eq!(e2e.llm.turn_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            e2e.count_inbound_with_body(unlinked_text).await,
            0,
            "an unlinked sender's ambient chatter must not be stored"
        );

        // 3. ADDRESSED message (a reply to one of the bot's messages) from the
        //    linked owner: the pipeline runs, and its prompt carries the
        //    ambient transcript captured in step 1.
        room.send_room_reply_to_bot(&owner, "what do you think of that session?")
            .await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "an addressed group message must reach the LLM in mentions mode"
        );

        let captured = e2e.llm.seen_turn_requests.lock().unwrap().join("\n");
        assert!(
            captured.contains("Recent group chat"),
            "addressed group turn must carry the ambient-transcript block"
        );
        assert!(
            captured.contains(ambient_text),
            "the ambient message stored in step 1 must appear in the transcript"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn all_mode_still_answers_unaddressed_group_messages() {
        let (e2e, room, owner) = room_in(GroupRespondMode::All).await;
        e2e.llm.set_turn_replies(&["Bien reçu."]);

        room.send_room(&owner, "how is my training load looking?")
            .await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "all mode (the default) must keep answering unaddressed group messages"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn group_respond_command_flips_mode_and_enforces_roles() {
        let (e2e, room, owner) = room_in(GroupRespondMode::All).await;
        e2e.llm.set_turn_replies(&["Jamais appelé."]);

        // A second linked member, auto-enrolled as a plain Member on first
        // contact with the room.
        let member = e2e.linked_member(true).await;

        let respond_mode_in_db = || async {
            e2e.resources
                .common
                .repos
                .groups
                .get_group(&room.group_id.to_string(), e2e.bot_tenant)
                .await
                .unwrap()
                .expect("group row must exist")
                .respond_mode
        };

        // A plain Member may not change the mode.
        room.send_room_slash(&member, "/group respond mentions")
            .await;
        assert_eq!(
            respond_mode_in_db().await,
            GroupRespondMode::All,
            "a plain member must not be able to flip the respond mode"
        );

        // The Owner flips to mentions.
        room.send_room_slash(&owner, "/group respond mentions")
            .await;
        assert_eq!(respond_mode_in_db().await, GroupRespondMode::Mentions);

        // ESCAPE HATCH: with the group now in mentions mode, the Owner's
        // UNADDRESSED slash command must still be honored — otherwise the
        // mode could never be reverted from inside the chat.
        room.send_room_slash(&owner, "/group respond all").await;
        assert_eq!(
            respond_mode_in_db().await,
            GroupRespondMode::All,
            "an unaddressed /group respond must work in mentions mode (escape hatch)"
        );

        // Command turns never reach the LLM.
        assert_eq!(e2e.llm.turn_calls.load(Ordering::SeqCst), 0);
    }
}
