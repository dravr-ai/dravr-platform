// ABOUTME: Integration tests for the messaging DM writing into the athlete's personal thread (carnet#840)
// ABOUTME: Pins the routing, the adoption of the thread the DM wrote into, the per-row channel, the agent pick, and where a notice answers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The app and the athlete's messaging DM are one conversation: the DM writes
//! into the personal thread Home shows, so a reply on either reads what was
//! said on the other.

#[cfg(feature = "client-messaging")]
mod dm_routing {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RouterLlm};
    use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
    use pierre_core::models::{AddMessageParams, TenantId};
    use pierre_core::transport::TransportPolicy;
    use pierre_services::personal_thread::{
        athlete_last_wrote_in_app, open_topic, resolve_dm_thread, resolve_personal_thread,
        PersonalForge, TopicOpening,
    };
    use serial_test::serial;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    /// How a missing personal thread is forged, on the server's own chain.
    fn forge(e2e: &CommandE2e) -> PersonalForge<'_> {
        PersonalForge {
            title_fallback: "Personal",
            llm_chain: &e2e.resources.common.config.llm_chain,
            guided_flow: false,
        }
    }

    async fn start() -> Arc<CommandE2e> {
        let llm = RouterLlm::new();
        let resources =
            create_test_server_resources_with_chat_provider(Arc::<RouterLlm>::clone(&llm))
                .await
                .unwrap();
        CommandE2e::start(resources, llm).await
    }

    /// The conversation the member's DM session names.
    async fn dm_thread(e2e: &CommandE2e, member: &Member) -> String {
        e2e.conversation_id(member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the DM session names a conversation")
    }

    /// One plain DM turn, waited for until its reply is ledgered.
    async fn dm_turn(e2e: &CommandE2e, member: &Member, text: &str) {
        let before = match e2e
            .session_id(member, member.home_tenant, &member.channel_user_id)
            .await
        {
            Some(session) => e2e.outbound_count_for_session(&session).await,
            None => 0,
        };
        e2e.send_dm(member, text).await;
        let session = e2e
            .session_id(member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the DM opened a session");
        e2e.wait_outbound_for_session(&session, before + 1).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_dm_writes_into_the_personal_thread_the_app_shows() {
        let e2e = start().await;
        let member = e2e.linked_member(false).await;
        let user = member.user_id.to_string();
        let personal = resolve_personal_thread(
            &e2e.resources.common.repos,
            &user,
            member.home_tenant,
            forge(&e2e),
        )
        .await
        .unwrap()
        .conversation
        .id;

        dm_turn(&e2e, &member, "How was my week?").await;

        assert_eq!(
            dm_thread(&e2e, &member).await,
            personal,
            "the DM writes into the thread the app shows"
        );
        let rows = e2e
            .resources
            .common
            .repos
            .chat
            .get_messages(&personal, &user, member.home_tenant)
            .await
            .unwrap();
        let question = rows
            .iter()
            .find(|row| row.role == "user" && row.content == "How was my week?")
            .expect("the DM's words are in the personal thread");
        assert_eq!(question.channel_type.as_deref(), Some("telegram"));
        assert!(
            rows.iter()
                .any(|row| row.role == "assistant"
                    && row.channel_type.as_deref() == Some("telegram")),
            "the reply names the channel it was written for: {rows:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_first_dm_with_no_thread_opens_the_personal_thread() {
        let e2e = start().await;
        let member = e2e.linked_member(false).await;
        let user = member.user_id.to_string();

        dm_turn(&e2e, &member, "Salut").await;

        let thread = dm_thread(&e2e, &member).await;
        let personal = e2e
            .resources
            .common
            .repos
            .chat
            .personal_conversation(&user, member.home_tenant)
            .await
            .unwrap()
            .expect("the first DM designated a personal thread");
        assert_eq!(
            personal.id, thread,
            "the DM's thread is the athlete's personal thread"
        );
    }

    /// Two own threads, the second one touched last.
    async fn two_threads(e2e: &CommandE2e, member: &Member, dm_last: bool) -> (String, String) {
        let chat = e2e.resources.common.repos.chat.as_ref();
        let user = member.user_id.to_string();
        let tenant = member.home_tenant;
        let in_app = chat
            .create_conversation(&user, tenant, "In the app", "m", None, None)
            .await
            .unwrap()
            .id;
        let dm = chat
            .create_conversation(&user, tenant, "On Telegram", "m", None, None)
            .await
            .unwrap()
            .id;
        chat.set_conversation_channel(&dm, &user, tenant, "telegram")
            .await
            .unwrap();
        // The thread written to last is the more recently active one.
        sleep(Duration::from_millis(20)).await;
        let last = if dm_last { &dm } else { &in_app };
        write(e2e, member, last, tenant, "latest words", Some("web")).await;
        (in_app, dm)
    }

    async fn write(
        e2e: &CommandE2e,
        member: &Member,
        conversation: &str,
        tenant: TenantId,
        content: &str,
        channel: Option<&str>,
    ) {
        e2e.resources
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: tenant,
                conversation_id: conversation,
                user_id: &member.user_id.to_string(),
                role: "user",
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: channel,
            })
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_dm_keeps_its_own_thread_when_it_is_the_athletes_newest() {
        let e2e = start().await;
        let member = e2e.linked_member(false).await;
        let (_, dm) = two_threads(&e2e, &member, true).await;
        let user = member.user_id.to_string();

        let thread = resolve_dm_thread(
            &e2e.resources.common.repos,
            &user,
            member.home_tenant,
            Some(&dm),
            forge(&e2e),
        )
        .await
        .unwrap();
        assert_eq!(
            thread.conversation.id, dm,
            "a Telegram history is not stranded"
        );
        assert!(thread.conversation.personal);
        assert!(!thread.forged);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_dm_moves_into_the_in_app_thread_when_that_one_is_newer() {
        let e2e = start().await;
        let member = e2e.linked_member(false).await;
        let (in_app, dm) = two_threads(&e2e, &member, false).await;
        let user = member.user_id.to_string();

        let thread = resolve_dm_thread(
            &e2e.resources.common.repos,
            &user,
            member.home_tenant,
            Some(&dm),
            forge(&e2e),
        )
        .await
        .unwrap();
        assert_eq!(thread.conversation.id, in_app);
        assert!(thread.conversation.personal);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_notice_answers_where_the_athlete_last_wrote() {
        let e2e = start().await;
        let member = e2e.linked_member(false).await;
        let repos = &e2e.resources.common.repos;
        let user = member.user_id.to_string();
        let tenant = member.home_tenant;
        let thread = resolve_personal_thread(repos, &user, tenant, forge(&e2e))
            .await
            .unwrap()
            .conversation
            .id;
        let in_app = || athlete_last_wrote_in_app(repos, &thread, &user, tenant);

        assert!(
            !in_app().await.unwrap(),
            "an empty thread has no app turn to answer"
        );
        write(
            &e2e,
            &member,
            &thread,
            tenant,
            "from telegram",
            Some("telegram"),
        )
        .await;
        assert!(!in_app().await.unwrap());
        write(
            &e2e,
            &member,
            &thread,
            tenant,
            "from the app",
            Some("mobile"),
        )
        .await;
        assert!(
            in_app().await.unwrap(),
            "the latest words were typed in the app"
        );
        write(
            &e2e,
            &member,
            &thread,
            tenant,
            "from telegram again",
            Some("telegram"),
        )
        .await;
        assert!(!in_app().await.unwrap());
        write(
            &e2e,
            &member,
            &thread,
            tenant,
            "before rows named a channel",
            None,
        )
        .await;
        assert!(
            !in_app().await.unwrap(),
            "a row with no channel reads as not the app, as a DM thread was answered"
        );
    }

    /// "+" with a specialist in the app makes that agent the athlete's pick,
    /// so the DM's per-turn rebind to the pick keeps the topic's agent.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_specialist_picked_for_a_topic_stays_bound_through_a_dm_turn() {
        let e2e = start().await;
        let member = e2e.linked_member(false).await;
        let repos = &e2e.resources.common.repos;
        let user = member.user_id.to_string();
        let tenant = member.home_tenant;
        let thread = resolve_personal_thread(repos, &user, tenant, forge(&e2e))
            .await
            .unwrap()
            .conversation;
        let agent = repos
            .agents
            .create(
                member.user_id,
                tenant,
                &CreateAgentRequest {
                    title: "Tempo Agent".to_owned(),
                    description: None,
                    system_prompt: "You are a tempo coach.".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
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

        open_topic(
            repos,
            &thread,
            &user,
            tenant,
            TopicOpening {
                agent_id: Some(&agent),
                guided_flow: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            repos
                .tenants
                .get_selected_agent(tenant, member.user_id)
                .await
                .unwrap()
                .as_deref(),
            Some(agent.as_str()),
            "the topic's agent is the athlete's pick"
        );

        dm_turn(&e2e, &member, "How was my week?").await;

        let after = repos
            .chat
            .get_conversation(&thread.id, &user, tenant)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.agent_id.as_deref(),
            Some(agent.as_str()),
            "the DM turn kept the agent the topic opened with"
        );
    }
}
