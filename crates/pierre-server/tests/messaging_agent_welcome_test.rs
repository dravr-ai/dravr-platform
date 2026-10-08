// ABOUTME: A messaging athlete who binds an agent hears its welcome as text, starters listed, after the confirmation
// ABOUTME: Drives /agent add and the proposal's numeric pick over a signed Telegram webhook and reads the ledger

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#735 on the messaging surfaces.
//!
//! A channel gets the agent's welcome as one more message after the command's
//! answer, its starter questions written out as a list — buttons would not fit
//! Telegram's 64-byte callback data. The same row is in the thread, so the app
//! draws it with buttons when the athlete opens the DM there.
//!
//! The proposal's "Reply with a number" is the messaging « Démarrer »: the pick
//! now binds the thread at once and the agent opens it, instead of the
//! selection waiting for the athlete's next message to reach the thread.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod messaging_welcome {
    use std::env;
    use std::slice;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use serde_json::json;
    use serial_test::serial;
    use uuid::Uuid;

    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RoomE2e, RouterLlm};
    use pierre_commands::agent_create::AGENT_PROPOSAL_ACTION;
    use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
    use pierre_core::models::groups::GroupRespondMode;
    use pierre_core::models::AGENT_WELCOME_FINISH_REASON;
    use pierre_database::repositories::{MessagingRepository, PendingGuardianAction};

    const TITLE: &str = "Tempo Agent";
    const STARTERS: [&str; 3] = [
        "Plan my tempo week",
        "Check my tempo pace",
        "How long should a tempo run be",
    ];

    async fn start() -> Arc<CommandE2e> {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["OK."]);
        e2e
    }

    /// An agent the member authored, with a role and three examples.
    async fn tempo_agent(e2e: &CommandE2e, m: &Member) -> String {
        e2e.resources
            .common
            .repos
            .agents
            .create(
                m.user_id,
                m.home_tenant,
                &CreateAgentRequest {
                    title: TITLE.to_owned(),
                    description: Some("Tempo runs for the marathon build".to_owned()),
                    system_prompt: "You are a tempo coach.".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: STARTERS.iter().map(|s| (*s).to_owned()).collect(),
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
            .to_string()
    }

    /// A member with a forged DM session, and that session's id and thread.
    async fn primed_member(e2e: &CommandE2e) -> (Member, String, String) {
        let member = e2e.linked_member(false).await;
        e2e.send_dm(&member, "/status").await;
        let chat = member.channel_user_id.clone();
        let session = e2e
            .session_id(&member, member.home_tenant, &chat)
            .await
            .expect("the prime forges a DM session");
        let conversation = e2e
            .conversation_id(&member, member.home_tenant, &chat)
            .await
            .expect("the DM session has a thread");
        e2e.wait_outbound_for_session(&session, 1).await;
        (member, session, conversation)
    }

    async fn welcome_rows(e2e: &CommandE2e, m: &Member, conversation: &str) -> Vec<String> {
        e2e.resources
            .common
            .repos
            .chat
            .get_messages(conversation, &m.user_id.to_string(), m.home_tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
            .map(|row| row.content)
            .collect()
    }

    /// A welcome as a channel gets it lists three starters: those ranked from
    /// the athlete's state lead (carnet#828), and the agent's first example
    /// fills a slot they leave. No postback reaches the text.
    fn assert_lists_starters(welcome: &str) {
        assert_eq!(welcome.matches("\n- ").count(), 3, "{welcome}");
        assert!(
            welcome.contains(STARTERS[0]),
            "{:?} missing from {welcome}",
            STARTERS[0]
        );
        assert!(
            !welcome.contains("uc:") && !welcome.contains("ex:"),
            "{welcome}"
        );
    }

    /// The last ledgered message lists the starters, after one naming the
    /// agent — the welcome follows the answer that bound it.
    fn assert_welcome_follows(bodies: &[String]) {
        let (welcome, before) = bodies.split_last().expect("the welcome was ledgered");
        assert_lists_starters(welcome);
        assert!(
            before.last().is_some_and(|reply| reply.contains(TITLE)),
            "the answer naming the agent precedes the welcome: {before:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn agent_add_in_a_dm_sends_the_welcome_after_the_answer() {
        let e2e = start().await;
        let (member, session, conversation) = primed_member(&e2e).await;
        let agent = tempo_agent(&e2e, &member).await;
        let baseline = e2e.outbound_count_for_session(&session).await;

        e2e.send_dm(&member, &format!("/agent add {agent}")).await;

        e2e.wait_outbound_for_session(&session, baseline + 2).await;
        assert_welcome_follows(&e2e.outbound_bodies_for_session(&session).await);
        let rows = welcome_rows(&e2e, &member, &conversation).await;
        assert_eq!(rows.len(), 1, "one welcome row in the DM thread");
        assert!(rows[0].contains(TITLE));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_numeric_pick_binds_the_thread_and_the_agent_opens_it() {
        let e2e = start().await;
        let (member, session, conversation) = primed_member(&e2e).await;
        let agent = tempo_agent(&e2e, &member).await;
        let db: &dyn MessagingRepository = &*e2e.resources.common.repos.messaging;
        db.mark_agent_proposal_sent(
            e2e.bot_tenant,
            "telegram",
            &member.channel_user_id,
            slice::from_ref(&agent),
        )
        .await
        .unwrap();
        let baseline = e2e.outbound_count_for_session(&session).await;

        e2e.send_dm(&member, "1").await;

        let thread = e2e
            .resources
            .common
            .repos
            .chat
            .get_conversation(
                &conversation,
                &member.user_id.to_string(),
                member.home_tenant,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            thread.agent_id.as_deref(),
            Some(agent.as_str()),
            "the pick binds the thread now, not on the next message"
        );
        e2e.wait_outbound_for_session(&session, baseline + 1).await;
        let rows = welcome_rows(&e2e, &member, &conversation).await;
        assert_eq!(rows.len(), 1, "the picked agent opened the thread");
        let bodies = e2e.outbound_bodies_for_session(&session).await;
        assert_lists_starters(bodies.last().unwrap());
    }

    /// Regression: the pick never spent the proposal, so every later bare
    /// number was another pick. With the pick now rebinding the thread and
    /// posting a welcome at once, the agent's own "how many gels did you
    /// take?" answered with "2" switched the thread to the proposal's second
    /// agent mid-conversation. Once picked, a bare number is conversation.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_second_bare_number_after_the_pick_reaches_the_model() {
        let e2e = start().await;
        let (member, session, conversation) = primed_member(&e2e).await;
        let picked = tempo_agent(&e2e, &member).await;
        let other = e2e
            .seed_coach_agent(member.user_id, member.home_tenant, "Gel Agent")
            .await
            .id
            .to_string();
        let db: &dyn MessagingRepository = &*e2e.resources.common.repos.messaging;
        db.mark_agent_proposal_sent(
            e2e.bot_tenant,
            "telegram",
            &member.channel_user_id,
            &[picked.clone(), other],
        )
        .await
        .unwrap();
        let baseline = e2e.outbound_count_for_session(&session).await;
        e2e.send_dm(&member, "1").await;
        // The pick's confirmation is not ledgered; its welcome is.
        e2e.wait_outbound_for_session(&session, baseline + 1).await;
        let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);

        e2e.send_dm(&member, "2").await;

        assert!(
            e2e.wait_llm_turns(turns_before + 1).await,
            "the second number is a coaching turn, not a pick"
        );
        let thread = e2e
            .resources
            .common
            .repos
            .chat
            .get_conversation(
                &conversation,
                &member.user_id.to_string(),
                member.home_tenant,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            thread.agent_id.as_deref(),
            Some(picked.as_str()),
            "the picked agent keeps the thread"
        );
        assert_eq!(
            welcome_rows(&e2e, &member, &conversation).await.len(),
            1,
            "no second agent opened the thread"
        );
        assert!(
            db.proposed_agent_ids(e2e.bot_tenant, "telegram", &member.channel_user_id)
                .await
                .unwrap()
                .is_empty(),
            "the answered proposal holds no offer"
        );
    }

    /// A room the owner's `/agent create confirm` binds a new agent into.
    const CHAT_ROOM_CREATE: i64 = -100_735_001;
    /// The drafted agent's role: only its welcome says it, so a ledger row
    /// carrying it is the welcome reaching the room.
    const DRAFT_ROLE: &str = "Hill repeats for the spring trail block";

    /// Regression: a command that binds an agent in a shared room but answers
    /// the caller privately (`/agent create confirm` is not room-visible) wrote
    /// the welcome row and its introduction-ledger row, then sent only the
    /// private answer. The room never heard the agent, and the ledger kept its
    /// first reply from introducing itself. The welcome is the room's: it goes
    /// to the room even when the answer does not.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn agent_create_in_a_room_sends_the_welcome_to_the_room() {
        let e2e = start().await;
        let owner = e2e.linked_member(false).await;
        let room = RoomE2e::bind_room(
            Arc::clone(&e2e),
            CHAT_ROOM_CREATE,
            GroupRespondMode::Mentions,
            &owner,
        )
        .await;
        let token = Uuid::new_v4().simple().to_string();
        e2e.resources
            .common
            .repos
            .guardian_actions
            .create_pending_action(
                &PendingGuardianAction {
                    id: token.clone(),
                    tenant_id: owner.home_tenant.to_string(),
                    user_id: owner.user_id.to_string(),
                    conversation_id: None,
                    tool_name: AGENT_PROPOSAL_ACTION.to_owned(),
                    arguments: json!({
                        "title": "Hill Agent",
                        "description": DRAFT_ROLE,
                        "system_prompt": "You coach hill repeats.",
                        "category": "training",
                        "tags": [],
                    }),
                    deny_reason: "awaiting_confirmation".to_owned(),
                },
                Utc::now() + Duration::minutes(10),
            )
            .await
            .unwrap();

        room.send_room_slash(&owner, &format!("/agent create confirm {token}"))
            .await;

        let conversation = room
            .conversation_id(&owner)
            .await
            .expect("the command opened the owner's room thread");
        let welcomes: Vec<String> = e2e
            .resources
            .common
            .repos
            .chat
            .get_messages(&conversation, &owner.user_id.to_string(), e2e.bot_tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
            .map(|row| row.content)
            .collect();
        assert_eq!(welcomes.len(), 1, "the new agent welcomed the room thread");
        assert!(welcomes[0].contains(DRAFT_ROLE), "{}", welcomes[0]);
        let session = room
            .session_id(&owner)
            .await
            .expect("the command forged a room session");
        e2e.wait_outbound_for_session(&session, 2).await;
        e2e.wait_outbound_containing(DRAFT_ROLE, 1).await;
    }
}
