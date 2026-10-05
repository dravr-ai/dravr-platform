// ABOUTME: Tests /reset (/nouveau, /new) — the catalogue matches it, and the rotation it performs
// ABOUTME: The confirmation speaks the stored locale, the session lands on a new thread, the old one is archived off the cap; a failed repoint rolls back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use dravr_canot::commands::{CommandMatcher, CommandRegistry};
use pierre_commands::parser::load_command_catalog;

use crate::helpers::command_e2e::commands_dir;

/// `/reset` is an ordinary catalogue command, so the matcher every surface
/// shares is what decides whether a message is one.
///
/// The negative cases are the point: a conversation the athlete meant to keep
/// must survive them saying the word.
#[test]
fn the_catalogue_matches_only_the_explicit_reset_forms() {
    let definitions = load_command_catalog(&commands_dir()).definitions;
    assert!(
        !definitions.is_empty(),
        "the commands/ catalogue must load — otherwise this test asserts nothing"
    );
    let mut registry = CommandRegistry::new();
    for definition in definitions {
        registry.register(definition);
    }
    let matcher = CommandMatcher::from_registry(&registry);

    for cmd in ["/reset", "/RESET", " /nouveau ", "/new", "/New"] {
        let parsed = matcher
            .try_match(cmd.trim(), &registry)
            .unwrap_or_else(|| panic!("{cmd:?} should match the reset command"));
        assert_eq!(parsed.name, "reset", "{cmd:?} matched {}", parsed.name);
    }

    for not in [
        "reset",
        "nouveau",
        "reset my training",
        "/resetx",
        "show me 2022",
    ] {
        assert!(
            matcher
                .try_match(not, &registry)
                .is_none_or(|p| p.name != "reset"),
            "{not:?} must NOT be treated as the reset command"
        );
    }
}

/// `/reset` over the real wire: what it says, and what it moves.
///
/// The locale tests read their expected rows from the registry rather than
/// hardcoding them, and pin that the `en` row differs from the `fr` one so a
/// registry fallback to the default cannot pass them vacuously.
#[cfg(feature = "client-messaging")]
mod reset_locale {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RouterLlm};
    use crate::helpers::notify_capture::{capture_logs, named, only};
    use dravr_canot::rich_text::{parse_markdown, render_rich_text};
    use pierre_commands::reset::ResetHandler;
    use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
    use pierre_config::constants::usage_quotas::DEFAULT_MAX_ACTIVE_CONVERSATIONS;
    use pierre_contremaitre::messaging_strings::{
        DEFAULT_LOCALE, KEY_ARCHIVED_CONVERSATION_QUOTA, KEY_NEW_CONVERSATION_TITLE_PREFIX,
        KEY_RESET_CONFIRM, KEY_RESET_QUOTA, KEY_RESET_WALK_INTERRUPTED,
    };
    use pierre_core::errors::{AppError, ErrorCode};
    use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
    use pierre_core::models::{MessageRecord, AGENT_WELCOME_FINISH_REASON};
    use pierre_core::transport::TransportPolicy;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_runtime_context::CommandCtx;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tracing::Level;

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
            "/reset must dispatch as a command, not be stored as a chat turn"
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
        // The catalogue rows carry inline markdown and the messaging egress
        // converts them into the channel's dialect, so the ledgered body is
        // the converted form — the note names `/pillars` as a code span.
        let expected = render_rich_text(&parse_markdown(&format!("{en_confirm}{en_note}")));
        let fr_note = render_rich_text(&parse_markdown(&fr_note));
        assert!(
            bodies.contains(&expected),
            "the ledgered /reset confirmation must end with the en interrupted-walk note {en_note:?}; ledgered after the prime: {bodies:?}"
        );
        assert!(
            !bodies.iter().any(|b| b.ends_with(&fr_note)),
            "an en athlete must not get the default-locale walk note: {bodies:?}"
        );
    }

    /// The rotation itself: the session ends up on a different conversation,
    /// and the one it left is still there.
    ///
    /// This is what a canned confirmation would not do — the reply can be
    /// perfect while nothing moved, which is exactly the state the command
    /// existed to fix.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_moves_the_session_onto_a_fresh_conversation_and_keeps_the_old_one() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let (member, session, baseline) = primed_en_member(&e2e).await;
        let before = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the primed session names its conversation");

        // Rename the thread being left, so the title assertion below can tell
        // "named itself" from "inherited what it replaced".
        e2e.resources
            .common
            .repos
            .chat
            .update_conversation_title(
                &before,
                &member.user_id.to_string(),
                member.home_tenant,
                "Ancienne discussion",
            )
            .await
            .unwrap();

        let _ = reset_bodies(&e2e, &member, &session, baseline).await;

        let after = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the session still names a conversation after the reset");
        assert_ne!(
            before, after,
            "the session must point at a different conversation after /reset"
        );

        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let archived = chat
            .get_conversation(&before, &user, member.home_tenant)
            .await
            .unwrap();
        assert!(
            archived.is_some(),
            "the conversation the athlete left must survive the reset, not be deleted"
        );

        let previous = archived.expect("the archived conversation reads back");
        let fresh = chat
            .get_conversation(&after, &user, member.home_tenant)
            .await
            .unwrap()
            .expect("the forged conversation is readable by its owner");
        assert_eq!(
            fresh.model, previous.model,
            "the fresh thread must run on the same model as the one it replaced"
        );
        assert_eq!(
            fresh.agent_id, previous.agent_id,
            "a reset changes the thread, not the coach the athlete trains with"
        );
        // The fresh thread names itself. Inheriting the old title left the
        // list with rows an athlete could not tell apart (observed on dev,
        // 2026-09-02). A thread with neither a room nor an agent — the
        // member's home tenant seeds no system agent — takes the dated stamp
        // in the athlete's `en` locale, never the channel's machine name.
        assert_eq!(
            previous.title, "Ancienne discussion",
            "the archived thread keeps the name it had"
        );
        assert!(
            fresh.agent_id.is_none(),
            "fixture precondition: no agent to name the fresh thread after"
        );
        let prefix = e2e.resources.mcp.messaging_strings_registry.render(
            KEY_NEW_CONVERSATION_TITLE_PREFIX,
            "en",
            &[],
        );
        assert!(
            fresh.title.starts_with(&format!("{prefix} ")),
            "a thread with nothing to be named after takes the dated stamp in the athlete's locale: {:?}",
            fresh.title
        );
        assert!(
            !fresh.title.starts_with("Messaging:"),
            "no thread is named after its channel any more: {:?}",
            fresh.title
        );
    }

    /// The primed member, their session and ledger baseline, and the
    /// conversation the session names — filled up so the member owns exactly
    /// `owned` active threads, that one included.
    async fn member_owning(e2e: &CommandE2e, owned: usize) -> (Member, String, i64, String) {
        let (member, session, baseline) = primed_en_member(e2e).await;
        let before = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the primed session names its conversation");
        // The primed DM thread is one owned row; fill the rest.
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        for i in 1..owned {
            chat.create_conversation(
                &user,
                member.home_tenant,
                &format!("Fill {i}"),
                "m",
                None,
                None,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            i64::try_from(owned).unwrap(),
            "fixture precondition: the athlete owns exactly {owned} active threads"
        );
        (member, session, baseline, before)
    }

    /// `/reset` is a swap: the thread it leaves is archived and stops holding
    /// a slot, so an athlete at the cap still gets a fresh thread. Before
    /// this, the archived thread kept counting and a reset at the cap was
    /// refused — the athlete could only start over by deleting history.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_at_the_cap_archives_the_old_thread_and_opens_a_fresh_one() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
        let (member, session, baseline, before) = member_owning(&e2e, cap).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();

        let bodies = reset_bodies(&e2e, &member, &session, baseline).await;
        let confirm =
            e2e.resources
                .mcp
                .messaging_strings_registry
                .render(KEY_RESET_CONFIRM, "en", &[]);
        assert!(
            bodies.iter().any(|b| b.contains(&confirm)),
            "a reset at the cap is confirmed; ledgered after the prime: {bodies:?}"
        );

        let after = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the session still names a conversation");
        assert_ne!(before, after, "the session moves onto the fresh thread");
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS,
            "the fresh thread takes the slot the archived one freed"
        );
        assert!(
            chat.get_conversation(&before, &user, member.home_tenant)
                .await
                .unwrap()
                .is_some(),
            "the archived thread stays readable"
        );
        assert_eq!(
            chat.list_conversations(
                &user,
                member.home_tenant,
                50,
                0,
                TransportPolicy::FirstPartyOnly
            )
            .await
            .unwrap()
            .items
            .len(),
            cap + 1,
            "the archived thread is still listed beside the fresh one"
        );
        // The thread left behind is archived, not merely out of view: a second
        // archive of it frees nothing.
        assert!(
            !chat
                .archive_conversation(&before, &user, member.home_tenant)
                .await
                .unwrap(),
            "the reset already archived the thread it left"
        );
    }

    /// Over the cap — it was lowered below what the athlete owns — the swap
    /// frees one slot and that is still not enough: `/reset` is refused, the
    /// session keeps its conversation, and that conversation is back in the
    /// active set, since the athlete is still writing to it.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_over_the_cap_is_refused_and_leaves_the_old_thread_active() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let owned = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap() + 1;
        let (member, session, baseline, before) = member_owning(&e2e, owned).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();

        let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);
        let ack = e2e.send_dm(&member, "/reset").await;
        assert_eq!(
            ack.messages_stored(),
            0,
            "/reset must dispatch as a command"
        );
        e2e.wait_outbound_for_session(&session, baseline + 1).await;
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            turns_before,
            "a refused /reset must not reach the LLM"
        );

        let after = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the session still names a conversation");
        assert_eq!(before, after, "a refused /reset must not move the session");
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            i64::try_from(owned).unwrap(),
            "a refused /reset forges nothing and restores the thread it archived"
        );
        assert!(
            chat.archive_conversation(&before, &user, member.home_tenant)
                .await
                .unwrap(),
            "the thread the session still names is active again"
        );

        let reg = &e2e.resources.mcp.messaging_strings_registry;
        let quota_line = reg.render(
            KEY_RESET_QUOTA,
            "en",
            &[&DEFAULT_MAX_ACTIVE_CONVERSATIONS.to_string()],
        );
        assert!(
            quota_line.contains(&DEFAULT_MAX_ACTIVE_CONVERSATIONS.to_string()),
            "the refusal names the cap: {quota_line:?}"
        );
        let confirm = reg.render(KEY_RESET_CONFIRM, "en", &[]);
        let bodies: Vec<String> = e2e
            .outbound_bodies_for_session(&session)
            .await
            .into_iter()
            .skip(usize::try_from(baseline).unwrap())
            .collect();
        assert!(
            bodies.iter().any(|b| b.contains(&quota_line)),
            "the athlete hears the quota line {quota_line:?}; ledgered after the prime: {bodies:?}"
        );
        assert!(
            !bodies.iter().any(|b| b.contains(&confirm)),
            "a refused /reset must not confirm a rotation; ledgered after the prime: {bodies:?}"
        );
    }

    /// The session's own thread archived out from under it, with `slots_left`
    /// free slots once it is: the member, session, baseline and that thread.
    async fn member_on_archived_thread(
        e2e: &CommandE2e,
        slots_left: usize,
    ) -> (Member, String, i64, String) {
        let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
        let (member, session, baseline, before) = member_owning(e2e, cap - slots_left).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        assert!(chat
            .archive_conversation(&before, &user, member.home_tenant)
            .await
            .unwrap());
        // Refill to the intended number of free slots.
        chat.create_conversation(&user, member.home_tenant, "Refill", "m", None, None)
            .await
            .unwrap();
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS - i64::try_from(slots_left).unwrap(),
            "fixture precondition: {slots_left} free slot(s), the session's thread archived"
        );
        (member, session, baseline, before)
    }

    /// The refusal an athlete hears for a turn into an archived thread at the
    /// cap, in English. It names the archive and the cap, and it is not the
    /// `/reset` line, which tells them to delete one "before starting another"
    /// when they started nothing.
    fn archived_quota_line(e2e: &CommandE2e) -> String {
        let strings = &e2e.resources.mcp.messaging_strings_registry;
        let cap = DEFAULT_MAX_ACTIVE_CONVERSATIONS.to_string();
        let line = strings.render(KEY_ARCHIVED_CONVERSATION_QUOTA, "en", &[&cap]);
        assert_eq!(
            line,
            format!(
                "This conversation is archived, and you already have {cap} conversations \
                 open — the maximum for your plan. Delete one in the app to continue here."
            )
        );
        assert_ne!(line, strings.render(KEY_RESET_QUOTA, "en", &[&cap]));
        line
    }

    /// A message on a messaging channel whose session names an archived
    /// thread takes that thread's slot back when one is free: the same choke
    /// point the app's turns pass, so the channel cannot route around it.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_dm_into_an_archived_thread_under_the_cap_reactivates_it() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let (member, session, baseline, before) = member_on_archived_thread(&e2e, 1).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let rows_before = chat
            .get_messages(&before, &user, member.home_tenant)
            .await
            .unwrap()
            .len();

        e2e.send_dm(&member, "How was my week?").await;
        e2e.wait_outbound_for_session(&session, baseline + 1).await;

        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS,
            "the session's thread holds a slot again"
        );
        assert!(
            chat.get_messages(&before, &user, member.home_tenant)
                .await
                .unwrap()
                .len()
                > rows_before,
            "the turn was written into the reactivated thread"
        );
    }

    /// At the cap, that message is refused before anything is written: the
    /// athlete hears the conversation-cap line, the model is never called,
    /// and the thread stays archived.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_dm_into_an_archived_thread_at_the_cap_is_refused_and_writes_nothing() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let (member, session, baseline, before) = member_on_archived_thread(&e2e, 0).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let rows_before = chat
            .get_messages(&before, &user, member.home_tenant)
            .await
            .unwrap()
            .len();
        let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);

        e2e.send_dm(&member, "How was my week?").await;
        e2e.wait_outbound_for_session(&session, baseline + 1).await;

        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            turns_before,
            "a refused turn must not reach the LLM"
        );
        assert_eq!(
            chat.get_messages(&before, &user, member.home_tenant)
                .await
                .unwrap()
                .len(),
            rows_before,
            "a refused turn persists no message row"
        );
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS
        );
        assert!(
            !chat
                .archive_conversation(&before, &user, member.home_tenant)
                .await
                .unwrap(),
            "the refused thread stays archived"
        );

        let quota_line = archived_quota_line(&e2e);
        let bodies: Vec<String> = e2e
            .outbound_bodies_for_session(&session)
            .await
            .into_iter()
            .skip(usize::try_from(baseline).unwrap())
            .collect();
        assert!(
            bodies.iter().any(|b| b.contains(&quota_line)),
            "the athlete hears the conversation-cap line {quota_line:?}; ledgered after the prime: {bodies:?}"
        );
    }

    /// On a channel too, a command decides for itself: at the cap, `/help`
    /// typed into the archived thread is answered and leaves it archived,
    /// while `/pillars` — which opens a walk the thread's next turns answer —
    /// is refused with the cap line before it runs.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn commands_into_an_archived_thread_at_the_cap_decide_per_command() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let (member, session, baseline, before) = member_on_archived_thread(&e2e, 0).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let quota_line = archived_quota_line(&e2e);

        e2e.send_dm(&member, "/help").await;
        e2e.wait_outbound_for_session(&session, baseline + 1).await;
        e2e.send_dm(&member, "/pillars").await;
        e2e.wait_outbound_for_session(&session, baseline + 2).await;

        let bodies: Vec<String> = e2e
            .outbound_bodies_for_session(&session)
            .await
            .into_iter()
            .skip(usize::try_from(baseline).unwrap())
            .collect();
        assert!(
            !bodies[0].contains(&quota_line),
            "/help is answered, not refused: {bodies:?}"
        );
        assert!(
            bodies.last().is_some_and(|b| b.contains(&quota_line)),
            "/pillars is refused with the cap line: {bodies:?}"
        );
        assert!(
            chat.is_conversation_archived(&before, &user, member.home_tenant)
                .await
                .unwrap(),
            "neither command reactivated the thread"
        );
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS
        );
    }

    /// A write the database itself refuses, for the two statements a `/reset`
    /// rollback turns on.
    ///
    /// The fault is a trigger in this test's own database, so the handler
    /// under test runs unmodified against the real repositories and receives
    /// the error they really produce — message prefix, error code and all. A
    /// repository double would have to stand in for `MessagingRepository` and
    /// `ChatRepository` (seventy-odd methods between them) and for the registry
    /// that holds them, to make two statements fail.
    #[derive(Clone, Copy)]
    enum Fault {
        /// Moving a messaging session onto another conversation.
        SessionRepoint,
        /// Deleting a conversation.
        ConversationDelete,
    }

    impl Fault {
        /// The text the refused statement's error carries.
        const fn marker(self) -> &'static str {
            match self {
                Self::SessionRepoint => "injected fault: session repoint",
                Self::ConversationDelete => "injected fault: conversation delete",
            }
        }

        /// The name of the trigger, and of the function behind it on
        /// `PostgreSQL`.
        const fn name(self) -> &'static str {
            match self {
                Self::SessionRepoint => "fault_session_repoint",
                Self::ConversationDelete => "fault_conversation_delete",
            }
        }

        /// The statement the trigger refuses, as a trigger event clause.
        const fn event(self) -> &'static str {
            match self {
                Self::SessionRepoint => "UPDATE OF pierre_conversation_id ON messaging_sessions",
                Self::ConversationDelete => "DELETE ON chat_conversations",
            }
        }

        /// Install the trigger; every later matching statement fails.
        async fn install(self, e2e: &CommandE2e) {
            let (name, event, marker) = (self.name(), self.event(), self.marker());
            match e2e.resources.agent.database.backend() {
                DatabaseBackend::SQLite(db) => {
                    sqlx::query(&format!(
                        "CREATE TRIGGER {name} BEFORE {event} \
                         BEGIN SELECT RAISE(ABORT, '{marker}'); END"
                    ))
                    .execute(db.pool())
                    .await
                    .unwrap();
                }
                #[cfg(feature = "postgresql")]
                DatabaseBackend::PostgreSQL(db) => {
                    sqlx::query(&format!(
                        "CREATE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql \
                         AS $$ BEGIN RAISE EXCEPTION '{marker}'; END $$"
                    ))
                    .execute(db.pool())
                    .await
                    .unwrap();
                    sqlx::query(&format!(
                        "CREATE TRIGGER {name} BEFORE {event} \
                         FOR EACH ROW EXECUTE FUNCTION {name}()"
                    ))
                    .execute(db.pool())
                    .await
                    .unwrap();
                }
            }
        }
    }

    /// The context the messaging ingress hands `/reset` for a DM from
    /// `member` on `conversation`, over the real server context.
    fn reset_ctx(e2e: &CommandE2e, member: &Member, conversation: &str) -> PlatformCommandContext {
        PlatformCommandContext {
            user_id: member.user_id,
            tenant_id: member.home_tenant,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/reset".to_owned(),
            ctx: Arc::<ServerContext>::clone(&e2e.resources) as Arc<dyn CommandCtx>,
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: Some(conversation.to_owned()),
            conversation_tenant_id: member.home_tenant,
            sender_id: Some(member.channel_user_id.clone()),
            rotation: ConversationRotation::default(),
            tool_runtime: Arc::<ServerContext>::clone(&e2e.resources),
        }
    }

    /// Run `/reset` with the session repoint refused and return the error it
    /// surfaced, having checked it is the repoint's own — the repository's
    /// message around the database's refusal — and that no rotation was
    /// recorded for the surface to follow.
    async fn reset_with_repoint_refused(
        e2e: &CommandE2e,
        member: &Member,
        conversation: &str,
    ) -> AppError {
        Fault::SessionRepoint.install(e2e).await;
        let ctx = reset_ctx(e2e, member, conversation);
        let error = ResetHandler
            .execute(&ctx)
            .await
            .expect_err("a /reset whose session cannot be repointed must fail");
        assert_eq!(error.code, ErrorCode::DatabaseError, "{error}");
        assert!(
            error
                .message
                .starts_with("Failed to update session conversation: "),
            "the error is the repoint's own: {error}"
        );
        assert!(
            error.message.contains(Fault::SessionRepoint.marker()),
            "the error carries the database's refusal: {error}"
        );
        assert_eq!(
            ctx.rotation.taken(),
            None,
            "a failed /reset must not tell the surface the athlete moved"
        );
        error
    }

    /// The ids of every thread the athlete is listed with, archived included.
    async fn listed_ids(e2e: &CommandE2e, member: &Member) -> Vec<String> {
        e2e.resources
            .common
            .repos
            .chat
            .list_conversations(
                &member.user_id.to_string(),
                member.home_tenant,
                50,
                0,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|c| c.id)
            .collect()
    }

    /// The session cannot be repointed after the old thread was archived and
    /// the fresh one forged: the athlete is still on the old thread, so the
    /// fresh one is removed and the old one holds its slot again.
    ///
    /// At the cap, so the count pins both halves: a forged thread left behind
    /// would read one over, an old thread left archived one under.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_whose_repoint_fails_removes_the_fresh_thread_and_restores_the_old_one() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
        let (member, _session, _baseline, before) = member_owning(&e2e, cap).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let mut listed_before = listed_ids(&e2e, &member).await;
        listed_before.sort();
        assert_eq!(listed_before.len(), cap, "fixture precondition");

        reset_with_repoint_refused(&e2e, &member, &before).await;

        let mut listed_after = listed_ids(&e2e, &member).await;
        listed_after.sort();
        assert_eq!(
            listed_after, listed_before,
            "the forged thread is gone and nothing else was removed"
        );
        assert!(
            !chat
                .is_conversation_archived(&before, &user, member.home_tenant)
                .await
                .unwrap(),
            "the thread the athlete is still on is active again"
        );
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS,
            "the athlete holds exactly the slots they held before /reset"
        );
        assert_eq!(
            e2e.conversation_id(&member, member.home_tenant, &member.channel_user_id)
                .await
                .as_deref(),
            Some(before.as_str()),
            "the session still names the thread it named before /reset"
        );
    }

    /// The same failed repoint, and the fresh thread cannot be deleted
    /// either: the orphan stays, a warning names it, and the old thread is
    /// restored regardless — without consulting the cap, which the orphan now
    /// puts the athlete one over — while the error returned is still the
    /// repoint's.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_whose_repoint_and_cleanup_both_fail_still_restores_the_old_thread() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;

        let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
        let (member, _session, _baseline, before) = member_owning(&e2e, cap).await;
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let listed_before = listed_ids(&e2e, &member).await;
        assert_eq!(listed_before.len(), cap, "fixture precondition");

        Fault::ConversationDelete.install(&e2e).await;
        let (lines, guard) = capture_logs();
        let error = reset_with_repoint_refused(&e2e, &member, &before).await;
        drop(guard);
        assert!(
            !error.message.contains(Fault::ConversationDelete.marker()),
            "the failed cleanup must not replace the repoint's error: {error}"
        );

        let orphans: Vec<String> = listed_ids(&e2e, &member)
            .await
            .into_iter()
            .filter(|id| !listed_before.contains(id))
            .collect();
        assert_eq!(
            orphans.len(),
            1,
            "the forged thread could not be removed and is still listed"
        );
        let warning = only(
            &lines,
            "Reset command: the thread a failed reset forged could not be removed",
        );
        assert_eq!(warning.level, Level::WARN);
        assert_eq!(warning.field("conversation_id"), orphans[0]);
        assert!(
            warning
                .field("error")
                .contains(Fault::ConversationDelete.marker()),
            "the warning carries why the cleanup failed: {:?}",
            warning.fields
        );

        let repoint = only(
            &lines,
            "Reset command: the messaging session could not be repointed",
        );
        assert_eq!(repoint.level, Level::WARN);
        assert_eq!(repoint.field("previous_conversation_id"), before);
        assert!(
            named(
                &lines,
                "Reset command: the thread a failed reset archived could not be restored",
            )
            .is_empty(),
            "the restore itself succeeded, so nothing reports it failing"
        );

        assert!(
            !chat
                .is_conversation_archived(&before, &user, member.home_tenant)
                .await
                .unwrap(),
            "the thread the athlete is still on is active again"
        );
        assert_eq!(
            chat.count_conversations(&user, member.home_tenant)
                .await
                .unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS + 1,
            "the old thread is restored past the cap the orphan already fills"
        );
        assert_eq!(
            e2e.conversation_id(&member, member.home_tenant, &member.channel_user_id)
                .await
                .as_deref(),
            Some(before.as_str()),
            "the session still names the thread it named before /reset"
        );
    }

    /// The starters the agent `/reset` carries onto the fresh thread offers.
    const WELCOME_STARTERS: [&str; 3] = [
        "Plan my tempo week",
        "Check my tempo pace",
        "How long should a tempo run be",
    ];

    /// `/reset` on an agent-bound DM (carnet#750): the athlete hears the
    /// confirmation, then the agent opening the fresh thread with its starters
    /// listed — the same welcome as any new thread — and the row is in the
    /// thread the session now names, not the one it left.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn reset_of_an_agent_thread_sends_the_agent_welcome_after_the_confirmation() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        let en_confirm = e2e
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_RESET_CONFIRM, "en");

        let (member, session, baseline) = primed_en_member(&e2e).await;
        let before = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the primed session names its conversation");
        // Retire the intake the DM opened with, so the confirmation is the
        // plain one and carries no interrupted-walk note.
        e2e.resources
            .common
            .repos
            .chat
            .set_conversation_onboarding_state(&before, None, member.home_tenant)
            .await
            .unwrap();
        let agent = e2e
            .resources
            .common
            .repos
            .agents
            .create(
                member.user_id,
                member.home_tenant,
                &CreateAgentRequest {
                    title: "Tempo Agent".to_owned(),
                    description: Some("Tempo runs for the marathon build".to_owned()),
                    system_prompt: "You are a tempo coach.".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: WELCOME_STARTERS.iter().map(|s| (*s).to_owned()).collect(),
                    startup_query: None,
                    data_requirements: None,
                    purpose: None,
                    when_to_use: None,
                    instructions: None,
                    example_inputs: None,
                    example_outputs: None,
                    success_criteria: None,
                    max_tool_iterations: None,
                },
            )
            .await
            .unwrap()
            .id
            .to_string();
        e2e.send_dm(&member, &format!("/agent add {agent}")).await;
        // The bind's own answer and welcome.
        let bound = e2e.wait_outbound_for_session(&session, baseline + 2).await;

        reset_bodies(&e2e, &member, &session, bound).await;
        // The welcome is ledgered after the confirmation `reset_bodies` waited on.
        e2e.wait_outbound_for_session(&session, bound + 2).await;
        let bodies: Vec<String> = e2e
            .outbound_bodies_for_session(&session)
            .await
            .into_iter()
            .skip(usize::try_from(bound).unwrap())
            .collect();
        assert_eq!(
            bodies.len(),
            2,
            "the confirmation, then the welcome: {bodies:?}"
        );
        assert_eq!(bodies[0], en_confirm, "the confirmation goes first");
        assert!(bodies[1].contains("Tempo Agent"), "{}", bodies[1]);
        for starter in WELCOME_STARTERS {
            assert!(
                bodies[1].contains(starter),
                "{starter:?} missing from {}",
                bodies[1]
            );
        }

        let fresh = e2e
            .conversation_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the session names the fresh thread");
        assert_ne!(fresh, before, "the session moved onto the fresh thread");
        let welcomes = |rows: Vec<MessageRecord>| {
            rows.into_iter()
                .filter(|row| row.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
                .count()
        };
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        assert_eq!(
            welcomes(
                chat.get_messages(&fresh, &user, member.home_tenant)
                    .await
                    .unwrap()
            ),
            1,
            "the agent opened the fresh thread"
        );
        assert_eq!(
            welcomes(
                chat.get_messages(&before, &user, member.home_tenant)
                    .await
                    .unwrap()
            ),
            1,
            "the thread left behind keeps only the bind's welcome"
        );
    }
}
