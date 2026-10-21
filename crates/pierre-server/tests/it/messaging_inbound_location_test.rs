// ABOUTME: A map pin an athlete shares on a channel is ledgered as a location and starts no coaching turn
// ABOUTME: Drives a Telegram location update through the real webhook, ingress and turn runner

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `MessageContent::Location` travels one way on the platform. No reply block
//! carries a place, so the egress never builds one; the variant's production
//! consumer is the ingress. canot parses the pin out of the webhook, and the
//! ingress stores it as a `location` row with no body and dispatches nothing
//! (`PersistOutcome::StoredNoDispatch`). canot's own suite proves the parse;
//! this proves the platform half, over the wire, so the consumer cannot
//! quietly disappear and leave the variant declared and read by nothing
//! (carnet#3).

#[cfg(feature = "client-messaging")]
mod inbound_location {
    use std::sync::Arc;

    use serial_test::serial;

    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, RouterLlm};

    /// What the athlete types after sharing the pin. Unique, so a coaching
    /// request that carries it is provably this text's turn and no other.
    const FOLLOW_UP: &str = "Quelle sortie autour de ce point demain matin ?";

    async fn start() -> Arc<CommandE2e> {
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["OK."]);
        e2e
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_shared_pin_is_stored_as_a_location_and_starts_no_turn() {
        let e2e = start().await;
        // A provider connection, so the onboarding gate lets the text turn
        // below reach the LLM — the control has to be able to start a turn.
        let member = e2e.linked_member(true).await;

        let ack = e2e.send_dm_location(&member, 48.8566, 2.3522).await;
        assert_eq!(
            ack.body["messages_received"], 1,
            "canot must parse the pin into one inbound message: {}",
            ack.body
        );
        assert_eq!(
            ack.messages_stored(),
            1,
            "the ingress must store the pin: {}",
            ack.body
        );

        // A DM lives under the athlete's own tenant.
        let session = e2e
            .session_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the pin must forge the athlete's DM session");
        assert_eq!(
            e2e.inbound_rows_for_session(member.home_tenant, &session)
                .await,
            vec![("location".to_owned(), None)],
            "the pin is ledgered as a location, with no text body to hand a model"
        );
        assert_eq!(
            e2e.outbound_count_for_session(&session).await,
            0,
            "nothing is sent back for a pin"
        );

        // The control: a text on the same conversation does start a turn.
        e2e.send_dm(&member, FOLLOW_UP).await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "the text after the pin must reach the LLM"
        );
        e2e.wait_outbound_for_session(&session, 1).await;

        // A conversation's turns are claimed oldest first: a younger turn
        // stays blocked until its predecessor finishes. Had the pin started a
        // turn, that turn would have run, and called the model, before the
        // text's. So once the text is answered, every coaching request seen
        // so far must be the text's own.
        let requests = e2e.llm.seen_turn_requests.lock().unwrap().clone();
        assert!(
            !requests.is_empty(),
            "the text's turn must have called the model"
        );
        assert!(
            requests.iter().all(|request| request.contains(FOLLOW_UP)),
            "a coaching request without the text is a turn the pin started: {requests:?}"
        );
        assert_eq!(
            e2e.outbound_count_for_session(&session).await,
            1,
            "the athlete hears one reply, to the text, and none to the pin"
        );

        let mut inbound = e2e
            .inbound_rows_for_session(member.home_tenant, &session)
            .await;
        inbound.sort();
        assert_eq!(
            inbound,
            vec![
                ("location".to_owned(), None),
                ("text".to_owned(), Some(FOLLOW_UP.to_owned())),
            ],
            "both inbound messages are on the ledger, each under its own content type"
        );
    }
}
