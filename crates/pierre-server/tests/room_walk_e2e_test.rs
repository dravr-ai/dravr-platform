// ABOUTME: Room guided walks through the REAL wire — webhook → ambient gate → pipeline → extraction
// ABOUTME: Pins the four e2e claims: home-tenant facts, probe advancement, walker-only exemption, no auto-start
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The handler-level tests (`room_guided_walk_test`) prove the component
//! contracts; these prove the PATH — a signed Telegram webhook, the ambient
//! gate, session resolution, the chat pipeline with a deterministic mock at
//! the one LLM seam, and the background extraction worker.
//!
//! Each test is built to FAIL under the regression it guards:
//!
//! 1. A reverted extraction-tenant stamp lands the answer under the bot
//!    tenant — both halves of the tenant assertion break.
//! 2. A resolver that stops advancing on the walker's turn freezes the probe
//!    ledger — the growth assertion breaks.
//! 3. A broken mentions-mode exemption silences the walker (no LLM turn), an
//!    over-broad one answers the peer (an extra LLM turn) — either direction
//!    breaks a count.
//! 4. A wholesale-deleted auto-start would pass the group half vacuously —
//!    the DM control exists so it cannot.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod room_walk_e2e {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RoomE2e, RouterLlm};
    use pierre_core::models::groups::{GroupRespondMode, GroupRole};
    use pierre_core::models::WalkAudience;
    use pierre_memory::FactSource;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    /// The `RawFact` array the extraction worker parses for the walker's
    /// answer turn. `stated_by: "user"` keeps every gate open; confidence
    /// sits above the 0.55 floor; the kind is overridden anyway — a guided
    /// answer's kind is forced by the topic it answers.
    const GOAL_FACT_JSON: &str = r#"[{"kind":"goal","subject":"walker","predicate":"objectif","object":"marathon en mai, 8 h par semaine","confidence":0.9,"stated_by":"user"}]"#;

    /// Turn replies: short (<40 chars) and free of advice cues, so neither
    /// playbook advice capture nor claim verification ever fires on them.
    const TURN_REPLIES: [&str; 3] = ["Q1 — ton objectif ?", "Q2 — ta dispo ?", "Q3 — noté."];

    const CHAT_WALK: i64 = -100_811_001;
    const CHAT_EXEMPT: i64 = -100_811_002;
    const CHAT_FRESH: i64 = -100_811_003;

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn guided_answer_lands_under_home_tenant_never_bot_tenant() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&TURN_REPLIES);
        e2e.llm.extraction_rule("marathon", GOAL_FACT_JSON);

        let walker = e2e.linked_member(true).await;
        let room = RoomE2e::bind_room(
            Arc::clone(&e2e),
            CHAT_WALK,
            GroupRespondMode::Mentions,
            &walker,
        )
        .await;

        // /calibrate in the room: slash bypasses the ambient gate, activates
        // the walk on the walker's own room conversation under the bot tenant.
        let ack = room.send_room_slash(&walker, "/calibrate").await;
        assert_eq!(
            ack.messages_stored(),
            0,
            "/calibrate must dispatch as a command"
        );
        let state = room
            .onboarding_state(&walker)
            .await
            .expect("the room walk must be active after /calibrate");
        assert_eq!(
            state.subject_user_id.as_deref(),
            Some(walker.user_id.to_string().as_str())
        );
        assert_eq!(state.audience, WalkAudience::Room);

        // Turn B — the walker's first answer-turn. It delivers probe 1; its
        // inbound answers no probe yet, so its extraction is ordinary
        // conversation and the mock answers it "[]".
        room.send_room(&walker, "salut coach, on y va").await;
        assert!(e2e.wait_llm_turns(1).await, "turn B must reach the LLM");
        wait_probes(&room, &walker, 1).await;
        assert!(
            e2e.facts_now(walker.home_tenant, &walker, FactSource::Onboarding)
                .await
                .is_empty(),
            "turn B answers no probe — nothing may land as an onboarding fact yet"
        );

        // Turn C — answers probe 1. Extraction must stamp the WALKER'S OWN
        // tenant: the fact content routes the mock to the goal-fact JSON.
        room.send_room(
            &walker,
            "Je veux courir un marathon en mai, 8 h par semaine",
        )
        .await;
        assert!(e2e.wait_llm_turns(2).await, "turn C must reach the LLM");

        let facts = e2e
            .wait_facts(walker.home_tenant, &walker, FactSource::Onboarding, 1)
            .await;
        assert!(
            facts.iter().any(|f| f.object.contains("marathon")),
            "the answer's content must be the fact that landed, got: {facts:?}"
        );
        // The other half of the tenant claim: NOTHING under the bot tenant. A
        // reverted stamp (conversation tenant again) fails both halves.
        assert!(
            e2e.facts_now(e2e.bot_tenant, &walker, FactSource::Onboarding)
                .await
                .is_empty(),
            "a room answer must never land in the bot tenant's fact space"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn walker_answer_advances_the_probe_ledger_through_the_real_turn() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&TURN_REPLIES);

        let walker = e2e.linked_member(true).await;
        let room = RoomE2e::bind_room(
            Arc::clone(&e2e),
            CHAT_WALK,
            GroupRespondMode::Mentions,
            &walker,
        )
        .await;

        room.send_room_slash(&walker, "/calibrate").await;

        room.send_room(&walker, "on y va").await;
        assert!(e2e.wait_llm_turns(1).await);
        let after_b = wait_probes(&room, &walker, 1).await;

        room.send_room(&walker, "surtout plus de volume, pas plus dur")
            .await;
        assert!(e2e.wait_llm_turns(2).await);
        let after_c = wait_probes(&room, &walker, 2).await;

        assert_ne!(
            after_c[0], after_c[1],
            "the second probe must be a different topic — a frozen resolver re-asks"
        );
        assert_eq!(&after_c[0], &after_b[0], "the ledger is append-only");

        // The probe question left as a real outbound ledger row — the record
        // of what the room was actually asked.
        e2e.wait_outbound_containing("Q2", 1).await;

        let state = room
            .onboarding_state(&walker)
            .await
            .expect("walk still active");
        assert_eq!(state.audience, WalkAudience::Room);
        assert_eq!(
            state.subject_user_id.as_deref(),
            Some(walker.user_id.to_string().as_str())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn mentions_mode_exempts_only_the_walker_from_ambient_capture() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&TURN_REPLIES);

        let walker = e2e.linked_member(true).await;
        let peer = e2e.linked_member(false).await;
        let room = RoomE2e::bind_room(
            Arc::clone(&e2e),
            CHAT_EXEMPT,
            GroupRespondMode::Mentions,
            &walker,
        )
        .await;
        room.add_member(&peer, GroupRole::Member).await;

        room.send_room_slash(&walker, "/calibrate").await;
        assert!(room.onboarding_state(&walker).await.is_some());

        // The walker's unaddressed answer must dispatch: an interview that
        // demanded an @-mention per answer would shed its athlete by Q2.
        room.send_room(&walker, "réponse du marcheur, sans mention")
            .await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "the walker's unaddressed answer must reach the LLM (exemption broken)"
        );
        let turns_after_walker = e2e.llm.turn_calls.load(Ordering::SeqCst);

        // The peer's unaddressed chatter stays ambient: captured for the
        // transcript, never dispatched. Bounded-wait negative on the count,
        // with the inbound capture as the positive proof the message arrived.
        let peer_text = "beau tempo ce matin tout le monde";
        room.send_room(&peer, peer_text).await;
        sleep(Duration::from_secs(4)).await;
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            turns_after_walker,
            "a non-walker's unaddressed message must stay ambient (exemption over-broad)"
        );
        assert_eq!(
            e2e.count_inbound_with_body(peer_text).await,
            1,
            "the ambient message must still be captured for the room transcript"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn fresh_group_conversation_never_autostarts_a_walk_but_a_dm_does() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["Bienvenue."]);

        // No provider and an empty dossier: exactly the athlete auto-start
        // exists for.
        let member = e2e.linked_member(false).await;
        let room =
            RoomE2e::bind_room(Arc::clone(&e2e), CHAT_FRESH, GroupRespondMode::All, &member).await;

        // First-ever room message forges the session + conversation; All mode
        // dispatches it without a mention.
        room.send_room(&member, "bonjour tout le monde").await;
        assert!(
            e2e.wait_llm_turns(1).await,
            "the All-mode room turn must run"
        );
        assert!(
            room.session_id(&member).await.is_some(),
            "the room session must exist — otherwise the negative below is vacuous"
        );
        assert!(
            room.onboarding_state(&member).await.is_none(),
            "a fresh GROUP conversation must never auto-start a subject-less walk"
        );

        // The DM control: the same member's first direct message still owes
        // them an interview. If auto-start were deleted wholesale rather than
        // gated to DMs, this half fails — the group half can never pass
        // vacuously.
        e2e.send_dm(&member, "bonjour").await;
        let mut dm_state = None;
        for _ in 0..100 {
            dm_state = e2e
                .onboarding_state(&member, member.home_tenant, &member.channel_user_id)
                .await;
            if dm_state.is_some() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            dm_state.is_some(),
            "a fresh DM conversation must still auto-start the guided flow"
        );
    }

    /// Poll (≤20s) until the walker's probe ledger holds `at_least` entries;
    /// returns the slugs.
    async fn wait_probes(room: &RoomE2e, walker: &Member, at_least: usize) -> Vec<String> {
        for _ in 0..200 {
            if let Some(state) = room.onboarding_state(walker).await {
                if state.probed.len() >= at_least {
                    return state
                        .probed
                        .iter()
                        .map(|slug| slug.as_str().to_owned())
                        .collect();
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        let found = room
            .onboarding_state(walker)
            .await
            .map_or(0, |s| s.probed.len());
        panic!("expected >= {at_least} delivered probes, found {found}");
    }
}
