// ABOUTME: Tests the /reset (/nouveau, /new) conversation-reset command — the matcher and the confirmation's locale
// ABOUTME: Only explicit slash forms reset; the ledgered confirmation speaks the athlete's stored locale, not the default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use pierre_core::models::messaging::MessageContent;
use pierre_mcp_server::services::messaging_ingress::is_reset_command;

fn text(s: &str) -> MessageContent {
    MessageContent::Text { body: s.to_owned() }
}

#[test]
fn reset_command_matches_only_explicit_slash_forms() {
    // Explicit slash forms (case-insensitive, surrounding whitespace ok).
    for cmd in ["/reset", "/RESET", " /nouveau ", "/new", "/New"] {
        assert!(
            is_reset_command(&text(cmd)),
            "{cmd:?} should be recognized as the reset command"
        );
    }
    // Bare words or near-misses must stay normal chat so we never reset a
    // conversation the user meant to keep (e.g. "reset my training plan").
    for not in [
        "reset",
        "nouveau",
        "reset my training",
        "/resetx",
        "show me 2022",
    ] {
        assert!(
            !is_reset_command(&text(not)),
            "{not:?} must NOT be treated as the reset command"
        );
    }
}

/// The confirmation over the real wire, for an athlete whose stored locale is
/// not the registry default.
///
/// `/reset` is answered ahead of the slash dispatcher, so it does not inherit
/// the dispatcher's locale resolution — the confirmation has to resolve the
/// athlete's locale itself, the way every other command reply does. Both
/// tests read the expected rows from the registry rather than hardcoding
/// them, and pin that the `en` row differs from the `fr` one so a registry
/// fallback to the default cannot pass them vacuously.
#[cfg(feature = "client-messaging")]
mod reset_locale {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RouterLlm};
    use pierre_contremaitre::messaging_strings::{
        DEFAULT_LOCALE, KEY_RESET_CONFIRM, KEY_RESET_WALK_INTERRUPTED,
    };
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// A fresh linked member whose profile locale is `en` — written through
    /// the same repository method `PUT /api/user/locale` uses — primed with
    /// `/status` so the DM session the reset confirmation is ledgered against
    /// exists. Returns the member, their session id and the ledger baseline.
    async fn primed_en_member(e2e: &CommandE2e) -> (Member, String, i64) {
        let member = e2e.linked_member(false).await;
        e2e.resources
            .common
            .repos
            .users
            .update_locale(member.user_id, "en")
            .await
            .unwrap();
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
        (member, session, baseline)
    }

    /// Send `/reset` and return the confirmation bodies ledgered after the
    /// baseline. Asserts the reset was recognised and never reached the model.
    async fn reset_bodies(
        e2e: &CommandE2e,
        member: &Member,
        session: &str,
        baseline: i64,
    ) -> Vec<String> {
        let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);
        let ack = e2e.send_dm(member, "/reset").await;
        assert_eq!(
            ack.messages_stored(),
            0,
            "/reset must be intercepted, not stored as a chat turn"
        );
        e2e.wait_outbound_for_session(session, baseline + 1).await;
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            turns_before,
            "/reset must not reach the LLM"
        );
        let bodies = e2e.outbound_bodies_for_session(session).await;
        bodies
            .into_iter()
            .skip(usize::try_from(baseline).unwrap())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_confirmation_speaks_the_athletes_locale() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let registry = &e2e.resources.mcp.messaging_strings_registry;
        let en_confirm = registry.get(KEY_RESET_CONFIRM, "en");
        let fr_confirm = registry.get(KEY_RESET_CONFIRM, DEFAULT_LOCALE);
        assert_ne!(
            en_confirm, fr_confirm,
            "the en and fr reset confirmations must differ or this test proves nothing"
        );

        let (member, session, baseline) = primed_en_member(&e2e).await;

        // A fresh DM conversation opens with the intake walk. Retire it so this
        // is the plain confirmation — the interrupted-walk note is the next
        // test's subject.
        let conversation = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the primed session names its conversation");
        e2e.resources
            .common
            .repos
            .chat
            .set_conversation_onboarding_state(&conversation, None, member.home_tenant)
            .await
            .unwrap();
        assert!(
            e2e.onboarding_state(&member, member.home_tenant, &member.channel_user_id)
                .await
                .is_none(),
            "no walk may be active before the plain-confirmation reset"
        );

        let bodies = reset_bodies(&e2e, &member, &session, baseline).await;
        assert!(
            bodies.contains(&en_confirm),
            "the ledgered /reset confirmation must be the en row {en_confirm:?}; ledgered after the prime: {bodies:?}"
        );
        assert!(
            !bodies.iter().any(|b| b.starts_with(&fr_confirm)),
            "an en athlete must not be answered in the default locale: {bodies:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_walk_interrupted_note_speaks_the_athletes_locale() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let registry = &e2e.resources.mcp.messaging_strings_registry;
        let en_confirm = registry.get(KEY_RESET_CONFIRM, "en");
        let en_note = registry.get(KEY_RESET_WALK_INTERRUPTED, "en");
        let fr_note = registry.get(KEY_RESET_WALK_INTERRUPTED, DEFAULT_LOCALE);
        assert_ne!(
            en_note, fr_note,
            "the en and fr interrupted-walk notes must differ or this test proves nothing"
        );

        let (member, session, baseline) = primed_en_member(&e2e).await;

        // The intake the fresh conversation opened with is the walk the reset
        // interrupts; the note only goes out when one is active.
        assert!(
            e2e.onboarding_state(&member, member.home_tenant, &member.channel_user_id)
                .await
                .is_some(),
            "a fresh DM conversation must have an active guided walk"
        );

        let bodies = reset_bodies(&e2e, &member, &session, baseline).await;
        let expected = format!("{en_confirm}{en_note}");
        assert!(
            bodies.contains(&expected),
            "the ledgered /reset confirmation must end with the en interrupted-walk note {en_note:?}; ledgered after the prime: {bodies:?}"
        );
        assert!(
            !bodies.iter().any(|b| b.ends_with(&fr_note)),
            "an en athlete must not get the default-locale walk note: {bodies:?}"
        );
    }
}
