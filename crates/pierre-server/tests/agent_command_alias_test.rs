// ABOUTME: /coach is a permanent synonym of /agent (D4) — this proves it resolves and does the identical work
// ABOUTME: Catalogue → matcher → in-app chat → signed Telegram webhook, plus the menu that publishes only the canonical name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#346: the coach→agent rename made `/agent` canonical and kept every
//! `/coach` spelling as a permanent alias — not a deprecation shim — so an
//! athlete's muscle memory and the `/coach` text already sitting in pinned
//! chat messages keep working. Roughly 37 assertions covered `/agent`; nothing
//! asserted that `/coach` still resolves, which is the entire justification
//! for keeping it.
//!
//! Resolution is three files deep and no existing test walks them together:
//! `commands/**/*.md` declares `command:` and `aliases:`, `pierre-commands`'
//! parser carries both into `CommandDefinition`, and dravr-canot's
//! `CommandRegistry` indexes every alias to the canonical name for
//! `CommandMatcher` to resolve — re-resolving once through the canonical
//! spelling so a subcommand typed after an alias reaches the right handler.
//!
//! So these walk the whole path: the catalogue, the matcher, the in-app chat
//! endpoint, and a real signed Telegram webhook. Every legacy spelling is
//! asserted to produce the SAME outcome as its canonical twin — same reply
//! text, same action values, same database row — never merely "no error". A
//! handler that silently did nothing would pass a no-error test and fail
//! every one of these.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod agent_alias {
    use std::collections::{BTreeMap, BTreeSet};
    use std::env;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use serde_json::json;
    use serial_test::serial;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    use crate::common::{
        create_test_server_resources, create_test_server_resources_with_chat_provider,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::coach_fixtures::{install_catalogue_coach, publish_catalogue_coach};
    use crate::helpers::command_e2e::{commands_dir, CommandE2e, Member, RouterLlm};
    use pierre_commands::load_command_catalog;
    use pierre_contremaitre::messaging_strings::{
        KEY_COACH_GROUP_UPDATED, KEY_COACH_REMOVED, KEY_UNKNOWN_COMMAND,
    };
    use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupInviteKind, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_core::models::{
        AddMessageParams, ConnectionType, Tenant, TenantId, User, UserStatus, COMMAND_FINISH_REASON,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::chat::{
        ChatRoutes, ConversationResponse, ReplyBlockResponse, TurnResponse,
    };
    use pierre_messaging::commands::{CommandDefinition, CommandMatcher, CommandRegistry};

    /// The persona the mock model proposes for `/agent create`, fenced the way
    /// real models fence JSON they were asked to return bare.
    const PROPOSAL_JSON: &str = "```json\n{\"title\":\"Coach Tempo\",\
\"description\":\"Tempo runs for the marathon build.\",\
\"system_prompt\":\"You are a tempo-run coach.\",\
\"category\":\"training\",\"tags\":[\"tempo\"]}\n```";

    /// Every spelling of the shelf listing, canonical first.
    const LIST_SPELLINGS: [&str; 5] =
        ["/agent", "/coach", "/coaches", "/agent list", "/coach list"];

    // ================================================================
    // Catalogue and matcher: what the three files actually declare
    // ================================================================

    /// The repository's own `commands/` tree, loaded exactly as the server
    /// loads it, as a registry plus the raw definitions.
    fn catalogue() -> (CommandRegistry, Vec<CommandDefinition>) {
        let definitions = load_command_catalog(&commands_dir()).definitions;
        assert!(
            definitions.len() > 10,
            "the commands/ catalogue should have loaded; found {} definitions at {}",
            definitions.len(),
            commands_dir().display()
        );
        let mut registry = CommandRegistry::new();
        for def in definitions.clone() {
            registry.register(def);
        }
        (registry, definitions)
    }

    /// Telegram menu entries are one token; a multi-word trigger has none.
    /// Mirrors `CommandRegistry::bot_command_list_described`'s own rule.
    fn single_token(trigger: &str) -> Option<String> {
        let name = trigger.trim_start_matches('/').to_owned();
        (!name.contains(' ')).then_some(name)
    }

    /// Every agent subcommand the catalogue declares keeps a `/coach` twin.
    ///
    /// Enumerated from `commands/**/*.md`, not from a list in this file: a
    /// seventh subcommand added canonically but never given its legacy
    /// spelling fails here rather than surprising an athlete.
    #[tokio::test]
    async fn every_agent_subcommand_declares_its_legacy_coach_spelling() {
        let (_registry, definitions) = catalogue();

        let declared: BTreeMap<String, String> = definitions
            .iter()
            .filter(|d| d.domain == "coach")
            .map(|d| (d.name.clone(), d.command.clone()))
            .collect();

        let expected: BTreeMap<String, String> = [
            ("coach-list", "/agent"),
            ("coach-add", "/agent add"),
            ("coach-create", "/agent create"),
            ("coach-remove", "/agent remove"),
            ("coach-assign", "/agent assign"),
            ("coach-invite", "/agent invite"),
        ]
        .into_iter()
        .map(|(name, command)| (name.to_owned(), command.to_owned()))
        .collect();
        assert_eq!(
            declared, expected,
            "the agent domain's canonical spellings changed; every new subcommand \
             needs its /coach twin below too"
        );

        for (name, command) in &declared {
            let twin = command.replacen("/agent", "/coach", 1);
            let def = definitions
                .iter()
                .find(|d| &d.name == name)
                .expect("the definition was just enumerated");
            assert!(
                def.aliases.contains(&twin),
                "{name} declares {command} but not its permanent alias {twin}; \
                 aliases were {:?}",
                def.aliases
            );
        }

        let list = definitions
            .iter()
            .find(|d| d.name == "coach-list")
            .expect("coach-list is in the catalogue");
        let aliases: BTreeSet<&str> = list.aliases.iter().map(String::as_str).collect();
        assert_eq!(
            aliases,
            ["/agent list", "/coach", "/coach list", "/coaches"]
                .into_iter()
                .collect::<BTreeSet<&str>>(),
            "the shelf listing answers to the bare /coach and /coaches too"
        );
    }

    /// The matcher resolves every legacy spelling to the canonical handler
    /// name, with the arguments intact.
    ///
    /// `/coaches invite` is the case the matcher's second pass exists for: the
    /// subcommand is registered under the canonical spelling only, so a match
    /// reached through a shorter alias has to be re-resolved or `invite` would
    /// be handed to the shelf listing as an argument.
    #[tokio::test]
    async fn the_matcher_resolves_every_legacy_spelling_to_the_canonical_handler() {
        let (registry, _definitions) = catalogue();
        let matcher = CommandMatcher::from_registry(&registry);

        let cases: [(&str, &str, &[&str]); 18] = [
            ("/agent", "coach-list", &[]),
            ("/coach", "coach-list", &[]),
            ("/coaches", "coach-list", &[]),
            ("/agent list", "coach-list", &[]),
            ("/coach list", "coach-list", &[]),
            (
                "/agent add @recovery-coach",
                "coach-add",
                &["@recovery-coach"],
            ),
            (
                "/coach add @recovery-coach",
                "coach-add",
                &["@recovery-coach"],
            ),
            (
                "/coaches add @recovery-coach",
                "coach-add",
                &["@recovery-coach"],
            ),
            ("/agent remove", "coach-remove", &[]),
            ("/coach remove", "coach-remove", &[]),
            ("/agent create", "coach-create", &[]),
            ("/coach create", "coach-create", &[]),
            ("/agent assign aaa bbb", "coach-assign", &["aaa", "bbb"]),
            ("/coach assign aaa bbb", "coach-assign", &["aaa", "bbb"]),
            ("/agent invite", "coach-invite", &[]),
            ("/coach invite", "coach-invite", &[]),
            ("/coaches invite", "coach-invite", &[]),
            (
                "/COACH ADD @recovery-coach",
                "coach-add",
                &["@recovery-coach"],
            ),
        ];

        for (typed, name, args) in cases {
            let parsed = matcher
                .try_match(typed, &registry)
                .unwrap_or_else(|| panic!("`{typed}` resolved to nothing"));
            assert_eq!(parsed.name, name, "`{typed}` reached the wrong handler");
            assert_eq!(parsed.args, args, "`{typed}` lost or gained arguments");
        }
    }

    /// The same guarantee for every alias in the catalogue, agent domain or
    /// not: an alias that stopped resolving is a command an athlete has typed
    /// before and would now be told does not exist.
    #[tokio::test]
    async fn every_catalogued_alias_resolves_to_its_own_definition() {
        let (registry, definitions) = catalogue();
        let matcher = CommandMatcher::from_registry(&registry);

        let mut checked = 0_usize;
        for def in &definitions {
            for alias in &def.aliases {
                let parsed = matcher.try_match(alias, &registry).unwrap_or_else(|| {
                    panic!("alias `{alias}` of {} resolved to nothing", def.name)
                });
                assert_eq!(
                    parsed.name, def.name,
                    "alias `{alias}` reached {} instead of {}",
                    parsed.name, def.name
                );
                checked += 1;
            }
        }
        assert_eq!(
            checked, 23,
            "the catalogue's alias inventory changed; read the new one and update this \
             count, so an alias silently dropped from a definition cannot pass here"
        );
    }

    /// A near-miss spelling resolves to nothing, so it reaches the athlete as
    /// "unknown command" rather than silently running the real one.
    #[tokio::test]
    async fn a_near_miss_spelling_resolves_to_nothing() {
        let (registry, _definitions) = catalogue();
        let matcher = CommandMatcher::from_registry(&registry);

        for typo in [
            "/agentt",
            "/coachh",
            "/coachess",
            "/agentlist",
            "/coach-add",
        ] {
            assert!(
                matcher.try_match(typo, &registry).is_none(),
                "`{typo}` must not resolve to a command"
            );
        }
        // The control: the matcher is not simply refusing everything.
        assert_eq!(
            matcher
                .try_match("/coach", &registry)
                .expect("/coach resolves")
                .name,
            "coach-list"
        );
    }

    // ================================================================
    // In-app chat: the same handler, the same reply, the same row
    // ================================================================

    async fn seed_user_tenant(
        resources: &Arc<ServerContext>,
        email: &str,
    ) -> (Uuid, TenantId, String) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Alias Test".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(chrono::Utc::now());

        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: "Alias Tenant".to_owned(),
            slug: format!("alias-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();
        resources
            .common
            .repos
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .unwrap();
        // The onboarding provider gate fires upstream of slash dispatch and
        // counts only non-synthetic connections.
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();

        let token = resources
            .auth
            .auth_manager
            .generate_token(&user, &resources.auth.jwks_manager)
            .unwrap();

        (user_id, tenant_id, format!("Bearer {token}"))
    }

    /// A coach the athlete owns outright, for the cases that need an id
    /// rather than a catalogue handle.
    async fn seed_coach(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
        title: &str,
    ) -> String {
        let request = CreateCoachRequest {
            title: title.to_owned(),
            description: Some(format!("{title} description.")),
            system_prompt: "You are a test persona.".to_owned(),
            category: CoachCategory::Training,
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
        };
        resources
            .common
            .repos
            .coaches
            .create(user_id, tenant_id, &request)
            .await
            .unwrap()
            .id
            .to_string()
    }

    /// Publish "Recovery Coach" under a fresh author and install it for the
    /// athlete; the installed copy answers to `@recovery-coach`.
    async fn install_recovery_coach(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> String {
        let (author_id, author_tenant, _auth) =
            seed_user_tenant(resources, &format!("alias-author-{user_id}@test.com")).await;
        let origin = publish_catalogue_coach(
            &resources.common.repos,
            author_id,
            author_tenant,
            "Recovery Coach",
            "You are the recovery coach.",
        )
        .await;
        let installed =
            install_catalogue_coach(&resources.common.repos, origin, user_id, tenant_id).await;
        assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));
        installed.id.to_string()
    }

    async fn create_conversation(router: axum::Router, auth: &str) -> String {
        let resp = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", auth)
            .json(&json!({"title": "Alias Test", "model": "gemini-1.5-flash"}))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::CREATED);
        let conv: ConversationResponse = resp.json();
        conv.id
    }

    /// Send one slash command on `conv_id` and return the turn.
    async fn send_command(
        router: axum::Router,
        auth: &str,
        conv_id: &str,
        text: &str,
    ) -> TurnResponse {
        let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
            .header("authorization", auth)
            .json(&json!({"content": text}))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{text}");
        let body: TurnResponse = resp.json();
        assert_eq!(
            body.assistant.finish_reason.as_deref(),
            Some(COMMAND_FINISH_REASON),
            "{text} must be answered as a command"
        );
        body
    }

    /// The postback values a turn's controls carry, in order.
    fn action_values(body: &TurnResponse) -> Vec<String> {
        body.assistant
            .blocks
            .iter()
            .find_map(|block| match block {
                ReplyBlockResponse::Actions { actions, .. } => {
                    Some(actions.iter().map(|a| a.value.clone()).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The label above a turn's controls, when it carried one.
    fn actions_title(body: &TurnResponse) -> Option<String> {
        body.assistant.blocks.iter().find_map(|block| match block {
            ReplyBlockResponse::Actions { title, .. } => title.clone(),
            _ => None,
        })
    }

    /// Render a messaging string in the platform default locale, which is the
    /// locale every user seeded here answers in.
    fn rendered(resources: &Arc<ServerContext>, key: &str, args: &[&str]) -> String {
        resources
            .mcp
            .messaging_strings_registry
            .render(key, "fr", args)
    }

    /// The coach the conversation row is bound to.
    async fn conversation_coach(
        resources: &Arc<ServerContext>,
        conv_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Option<String> {
        resources
            .common
            .repos
            .chat
            .get_conversation(conv_id, &user_id.to_string(), tenant_id)
            .await
            .unwrap()
            .expect("the conversation exists")
            .coach_id
    }

    /// The athlete's selection pointer.
    async fn selected_coach(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Option<String> {
        resources
            .common
            .repos
            .tenants
            .get_selected_coach(tenant_id, user_id)
            .await
            .unwrap()
    }

    /// The coach a group is pointed at.
    async fn group_coach(
        resources: &Arc<ServerContext>,
        group_id: Uuid,
        tenant_id: TenantId,
    ) -> String {
        resources
            .common
            .repos
            .groups
            .get_group(&group_id.to_string(), tenant_id)
            .await
            .unwrap()
            .expect("the group exists")
            .coach_id
    }

    /// A group named `name` owned by `user_id`, pointed at `coach_id`.
    ///
    /// Two groups may share a name here on purpose: the assignment reply
    /// renders the group's NAME, so twin names make the canonical and legacy
    /// replies comparable byte for byte while each acts on its own row.
    async fn seed_group(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
        name: &str,
        coach_id: &str,
    ) -> Uuid {
        let now = chrono::Utc::now();
        let group_id = Uuid::new_v4();
        let group = CoachingGroup {
            id: group_id,
            tenant_id: tenant_id.to_string(),
            name: name.to_owned(),
            description: None,
            coach_id: coach_id.to_owned(),
            owner_id: user_id,
            coach_user_id: None,
            peer_data_sharing: false,
            respond_mode: GroupRespondMode::default(),
            max_members: 20,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &group)
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant_id.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
        group_id
    }

    /// A fresh conversation bound to `group_id`, the way a group chat is.
    async fn group_conversation(
        resources: &Arc<ServerContext>,
        router: axum::Router,
        auth: &str,
        tenant_id: TenantId,
        group_id: Uuid,
    ) -> String {
        let conv_id = create_conversation(router, auth).await;
        resources
            .common
            .repos
            .chat
            .set_conversation_group_id(&conv_id, Some(&group_id.to_string()), tenant_id)
            .await
            .unwrap();
        conv_id
    }

    /// Write a coaching exchange into `conv_id`, so `/agent create` has
    /// something to draft a persona from.
    async fn seed_coaching_exchange(
        resources: &Arc<ServerContext>,
        conv_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) {
        let user_id = user_id.to_string();
        for (role, content) in [
            (
                "user",
                "Je prépare un marathon, comment placer mes sorties tempo ?",
            ),
            ("assistant", "Une sortie tempo par semaine, le jeudi."),
        ] {
            resources
                .common
                .repos
                .chat
                .add_message(&AddMessageParams {
                    tenant_id,
                    conversation_id: conv_id,
                    user_id: &user_id,
                    role,
                    content,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                })
                .await
                .unwrap();
        }
    }

    /// The shelf reads identically under all five spellings — same text, same
    /// card title, same buttons — and the content is real: the installed
    /// coach's handle and the canonical `/agent add` postback.
    #[tokio::test]
    async fn the_shelf_reads_the_same_under_every_spelling() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "alias-list@test.com").await;
        install_recovery_coach(&resources, user_id, tenant_id).await;
        let own = seed_coach(&resources, user_id, tenant_id, "My Own Agent").await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let conv_id = create_conversation(router.clone(), &auth).await;

        let mut answers = Vec::new();
        for spelling in LIST_SPELLINGS {
            let body = send_command(router.clone(), &auth, &conv_id, spelling).await;
            answers.push((
                spelling,
                body.assistant.message.content.clone(),
                actions_title(&body),
                action_values(&body),
            ));
        }

        let (_, canonical_text, canonical_title, canonical_values) = answers[0].clone();
        assert!(
            canonical_text.contains("@recovery-coach") && canonical_text.contains("My Own Agent"),
            "the shelf must name the athlete's coaches, or every comparison below is vacuous:\n\
             {canonical_text}"
        );
        assert!(
            canonical_title.is_some(),
            "the shelf card carries a title above its buttons"
        );
        assert!(
            canonical_values.contains(&"/agent add @recovery-coach".to_owned())
                && canonical_values.contains(&format!("/agent add {own}")),
            "the buttons carry the canonical spelling: {canonical_values:?}"
        );

        for (spelling, text, title, values) in &answers[1..] {
            assert_eq!(
                *text, canonical_text,
                "`{spelling}` answered differently from `/agent`"
            );
            assert_eq!(*title, canonical_title, "`{spelling}` card title differs");
            assert_eq!(
                *values, canonical_values,
                "`{spelling}` offered different buttons"
            );
        }
    }

    /// `/coach add @handle` binds exactly what `/agent add @handle` binds: the
    /// same confirmation text, the same `chat_conversations.coach_id`, the
    /// same selection pointer.
    #[tokio::test]
    async fn binding_an_agent_works_under_both_spellings() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "alias-add@test.com").await;
        let installed = install_recovery_coach(&resources, user_id, tenant_id).await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let canonical_conv = create_conversation(router.clone(), &auth).await;
        let legacy_conv = create_conversation(router.clone(), &auth).await;

        let canonical = send_command(
            router.clone(),
            &auth,
            &canonical_conv,
            "/agent add @recovery-coach",
        )
        .await;
        let legacy = send_command(
            router.clone(),
            &auth,
            &legacy_conv,
            "/coach add @recovery-coach",
        )
        .await;

        assert!(
            canonical
                .assistant
                .message
                .content
                .contains("Recovery Coach"),
            "the confirmation names the agent: {}",
            canonical.assistant.message.content
        );
        assert_eq!(
            legacy.assistant.message.content, canonical.assistant.message.content,
            "/coach add answered differently from /agent add"
        );
        assert_eq!(
            conversation_coach(&resources, &canonical_conv, user_id, tenant_id)
                .await
                .as_deref(),
            Some(installed.as_str()),
            "/agent add bound its conversation"
        );
        assert_eq!(
            conversation_coach(&resources, &legacy_conv, user_id, tenant_id)
                .await
                .as_deref(),
            Some(installed.as_str()),
            "/coach add bound its conversation to the same installed copy"
        );
        assert_eq!(
            selected_coach(&resources, user_id, tenant_id)
                .await
                .as_deref(),
            Some(installed.as_str()),
            "both spellings move the selection pointer"
        );
    }

    /// `/coach remove` detaches what `/agent remove` detaches, and says the
    /// same catalogued sentence naming the agent.
    #[tokio::test]
    async fn detaching_an_agent_works_under_both_spellings() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, auth) =
            seed_user_tenant(&resources, "alias-remove@test.com").await;
        install_recovery_coach(&resources, user_id, tenant_id).await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let canonical_conv = create_conversation(router.clone(), &auth).await;
        let legacy_conv = create_conversation(router.clone(), &auth).await;
        for conv in [&canonical_conv, &legacy_conv] {
            send_command(router.clone(), &auth, conv, "/agent add @recovery-coach").await;
        }

        let canonical = send_command(router.clone(), &auth, &canonical_conv, "/agent remove").await;
        let legacy = send_command(router.clone(), &auth, &legacy_conv, "/coach remove").await;

        let expected = rendered(&resources, KEY_COACH_REMOVED, &["Recovery Coach"]);
        assert_eq!(
            canonical.assistant.message.content, expected,
            "/agent remove renders the catalogued confirmation"
        );
        assert_eq!(
            legacy.assistant.message.content, expected,
            "/coach remove renders the very same sentence"
        );
        assert_eq!(
            conversation_coach(&resources, &canonical_conv, user_id, tenant_id).await,
            None
        );
        assert_eq!(
            conversation_coach(&resources, &legacy_conv, user_id, tenant_id).await,
            None,
            "/coach remove actually detached the row"
        );
        assert_eq!(
            selected_coach(&resources, user_id, tenant_id).await,
            None,
            "the selection pointer is cleared too"
        );
    }

    /// `/coach assign` points a group at an agent exactly as `/agent assign`
    /// does. Twin group names make the two replies comparable byte for byte
    /// while each call acts on its own group row.
    #[tokio::test]
    async fn assigning_an_agent_to_a_group_works_under_both_spellings() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, auth) =
            seed_user_tenant(&resources, "alias-assign@test.com").await;
        let starting = seed_coach(&resources, user_id, tenant_id, "Starting Agent").await;
        let target = seed_coach(&resources, user_id, tenant_id, "Assignable Agent").await;
        let canonical_group =
            seed_group(&resources, user_id, tenant_id, "Twin Group", &starting).await;
        let legacy_group =
            seed_group(&resources, user_id, tenant_id, "Twin Group", &starting).await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let conv_id = create_conversation(router.clone(), &auth).await;

        let canonical = send_command(
            router.clone(),
            &auth,
            &conv_id,
            &format!("/agent assign {target} {canonical_group}"),
        )
        .await;
        let legacy = send_command(
            router.clone(),
            &auth,
            &conv_id,
            &format!("/coach assign {target} {legacy_group}"),
        )
        .await;

        let expected = rendered(
            &resources,
            KEY_COACH_GROUP_UPDATED,
            &["Assignable Agent", "Twin Group"],
        );
        assert_eq!(canonical.assistant.message.content, expected);
        assert_eq!(
            legacy.assistant.message.content, expected,
            "/coach assign answered differently from /agent assign"
        );
        assert_eq!(
            group_coach(&resources, canonical_group, tenant_id).await,
            target
        );
        assert_eq!(
            group_coach(&resources, legacy_group, tenant_id).await,
            target,
            "/coach assign actually moved its group's coach"
        );
    }

    /// `/coach invite` issues the same human-coach invite `/agent invite`
    /// issues — a second `group_invites` row of kind `Coach`, and a body that
    /// differs from the canonical one only by its own code.
    ///
    /// This is the one B-sense subcommand in the tree: the invite attaches a
    /// human coach, and only the command's spelling was renamed.
    #[tokio::test]
    async fn the_human_coach_invite_is_issued_under_both_spellings() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, auth) =
            seed_user_tenant(&resources, "alias-invite@test.com").await;
        let starting = seed_coach(&resources, user_id, tenant_id, "Starting Agent").await;
        let group_id = seed_group(&resources, user_id, tenant_id, "Invite Group", &starting).await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let conv_id =
            group_conversation(&resources, router.clone(), &auth, tenant_id, group_id).await;

        let canonical = send_command(router.clone(), &auth, &conv_id, "/agent invite").await;
        let legacy = send_command(router.clone(), &auth, &conv_id, "/coach invite").await;

        let invites = resources
            .common
            .repos
            .groups
            .list_invites(&group_id.to_string())
            .await
            .unwrap();
        assert_eq!(
            invites.len(),
            2,
            "each spelling files its own invite: {invites:?}"
        );
        for invite in &invites {
            assert_eq!(
                invite.kind,
                GroupInviteKind::Coach,
                "both spellings file a human-coach invite, not an athlete one"
            );
            assert_eq!(invite.created_by, user_id);
        }

        let body_of = |text: &str| -> String {
            let code = invites
                .iter()
                .map(|i| i.code.as_str())
                .find(|code| text.contains(code))
                .unwrap_or_else(|| panic!("the reply carries no invite code:\n{text}"));
            assert!(
                text.contains("Invite Group"),
                "the reply names the group:\n{text}"
            );
            text.replace(code, "<code>")
        };
        assert_eq!(
            body_of(&legacy.assistant.message.content),
            body_of(&canonical.assistant.message.content),
            "/coach invite answered differently from /agent invite"
        );
    }

    /// `/coach create` drafts what `/agent create` drafts, and — the point of
    /// D4 — the confirm button it hands back carries the CANONICAL spelling,
    /// which then really creates the agent.
    #[tokio::test]
    async fn drafting_an_agent_works_under_both_spellings_and_confirms_canonically() {
        let llm = RouterLlm::new();
        llm.set_turn_replies(&[PROPOSAL_JSON]);
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let (user_id, tenant_id, auth) =
            seed_user_tenant(&resources, "alias-create@test.com").await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let canonical_conv = create_conversation(router.clone(), &auth).await;
        let legacy_conv = create_conversation(router.clone(), &auth).await;
        for conv in [&canonical_conv, &legacy_conv] {
            seed_coaching_exchange(&resources, conv, user_id, tenant_id).await;
        }

        let canonical = send_command(router.clone(), &auth, &canonical_conv, "/agent create").await;
        let legacy = send_command(router.clone(), &auth, &legacy_conv, "/coach create").await;

        assert!(
            canonical.assistant.message.content.contains("Coach Tempo"),
            "the draft card shows the proposed title: {}",
            canonical.assistant.message.content
        );
        assert_eq!(actions_title(&legacy), actions_title(&canonical));

        // Each draft is parked behind its own single-use token, so the two
        // cards can only be compared once that token is factored out.
        let confirm_of = |turn: &TurnResponse| -> (String, String) {
            let values = action_values(turn);
            assert_eq!(values.len(), 2, "create and discard: {values:?}");
            let confirm = values[0].clone();
            let token = confirm
                .strip_prefix("/agent create confirm ")
                .unwrap_or_else(|| {
                    panic!("the confirm button must carry the canonical spelling, got {confirm}")
                })
                .to_owned();
            assert_eq!(
                values[1],
                format!("/deny {token}"),
                "both buttons carry the same token"
            );
            (confirm, token)
        };
        let (_, canonical_token) = confirm_of(&canonical);
        let (confirm, legacy_token) = confirm_of(&legacy);
        assert_eq!(
            legacy
                .assistant
                .message
                .content
                .replace(&legacy_token, "<token>"),
            canonical
                .assistant
                .message
                .content
                .replace(&canonical_token, "<token>"),
            "/coach create drafted a different card"
        );
        assert!(
            legacy
                .assistant
                .message
                .content
                .contains("/agent create confirm "),
            "a draft reached through the legacy spelling still spells the canonical \
             command in its prose: {}",
            legacy.assistant.message.content
        );

        assert_eq!(
            resources
                .common
                .repos
                .coaches
                .count(user_id, tenant_id)
                .await
                .unwrap(),
            0,
            "drafting creates nothing"
        );
        let confirmed = send_command(router, &auth, &legacy_conv, &confirm).await;
        assert!(
            confirmed.assistant.message.content.contains("@coach-tempo"),
            "confirming teaches the new agent's handle: {}",
            confirmed.assistant.message.content
        );
        assert_eq!(
            resources
                .common
                .repos
                .coaches
                .count(user_id, tenant_id)
                .await
                .unwrap(),
            1,
            "the legacy spelling's draft really created the agent"
        );
        assert!(
            conversation_coach(&resources, &legacy_conv, user_id, tenant_id)
                .await
                .is_some(),
            "the new agent answers in the thread it was drafted from"
        );
    }

    /// A near-miss spelling is refused in chat and changes nothing: it is the
    /// catalogued "unknown command" body, not a silently-empty shelf.
    #[tokio::test]
    async fn a_near_miss_spelling_is_refused_in_chat_and_binds_nothing() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "alias-typo@test.com").await;
        install_recovery_coach(&resources, user_id, tenant_id).await;

        let router = ChatRoutes::routes(Arc::clone(&resources));
        let conv_id = create_conversation(router.clone(), &auth).await;
        let shelf = send_command(router.clone(), &auth, &conv_id, "/agent").await;

        let unknown = rendered(&resources, KEY_UNKNOWN_COMMAND, &[]);
        for typo in ["/agentt", "/coachh"] {
            let body = send_command(router.clone(), &auth, &conv_id, typo).await;
            assert_eq!(
                body.assistant.message.content, unknown,
                "`{typo}` must be answered as an unknown command"
            );
            assert_ne!(
                body.assistant.message.content, shelf.assistant.message.content,
                "`{typo}` must not fall through to the shelf"
            );
            assert!(
                action_values(&body).is_empty(),
                "`{typo}` offers no buttons"
            );
            assert!(actions_title(&body).is_none(), "`{typo}` renders no card");
            assert_eq!(body.telemetry.model, "command");
        }
        assert_eq!(
            conversation_coach(&resources, &conv_id, user_id, tenant_id).await,
            None,
            "a typo bound nothing"
        );
        assert_eq!(selected_coach(&resources, user_id, tenant_id).await, None);
    }

    // ================================================================
    // The wire: a real athlete typing /coach into Telegram
    // ================================================================

    /// One slash command over a signed Telegram webhook; returns the body the
    /// athlete was actually told, read back from the outbound ledger.
    async fn dm_reply(
        e2e: &CommandE2e,
        member: &Member,
        session: &str,
        sent: &mut i64,
        text: &str,
    ) -> String {
        let ack = e2e.send_dm(member, text).await;
        assert_eq!(
            ack.messages_stored(),
            0,
            "`{text}` was not recognised as a command in a DM"
        );
        *sent += 1;
        e2e.wait_outbound_for_session(session, *sent).await;
        let bodies = e2e.outbound_bodies_for_session(session).await;
        assert_eq!(
            i64::try_from(bodies.len()).unwrap(),
            *sent,
            "one ledgered reply per command, after `{text}`"
        );
        bodies.last().unwrap().clone()
    }

    /// The coach bound to the member's DM conversation.
    async fn dm_conversation_coach(e2e: &CommandE2e, member: &Member) -> Option<String> {
        let conversation = e2e
            .conversation_id(member, member.home_tenant, &member.channel_user_id)
            .await?;
        e2e.resources
            .common
            .repos
            .chat
            .get_conversation(
                &conversation,
                &member.user_id.to_string(),
                member.home_tenant,
            )
            .await
            .ok()
            .flatten()?
            .coach_id
    }

    /// The whole alias contract over the real wire: a signed Telegram webhook
    /// into the ingress, the slash dispatcher, and the outbound ledger.
    ///
    /// The catalogue-driven smoke (`command_smoke_e2e_test`) iterates
    /// `def.command` — the canonical spelling only — so this is the only place
    /// an alias is proven to survive the wire rather than just the matcher.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_telegram_dm_answers_the_legacy_spelling_identically() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        let e2e = CommandE2e::start(resources, llm).await;
        e2e.llm.set_turn_replies(&["OK."]);

        let author = e2e.linked_member(false).await;
        let member = e2e.linked_member(false).await;
        let origin = publish_catalogue_coach(
            &e2e.resources.common.repos,
            author.user_id,
            author.home_tenant,
            "Recovery Coach",
            "You are the recovery coach.",
        )
        .await;
        let installed = install_catalogue_coach(
            &e2e.resources.common.repos,
            origin,
            member.user_id,
            member.home_tenant,
        )
        .await;
        assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));
        let installed_id = installed.id.to_string();

        // The prime forges the DM session the ledger attaches replies to.
        e2e.send_dm(&member, "/status").await;
        let session = e2e
            .session_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the prime forges a DM session");
        let mut sent = e2e.wait_outbound_for_session(&session, 1).await;
        let turns_before = e2e.llm.turn_calls.load(Ordering::SeqCst);

        // Every spelling of the shelf answers the same body.
        let mut shelf = Vec::new();
        for spelling in LIST_SPELLINGS {
            shelf.push((
                spelling,
                dm_reply(&e2e, &member, &session, &mut sent, spelling).await,
            ));
        }
        assert!(
            shelf[0].1.contains("@recovery-coach"),
            "the shelf names the installed agent over the wire:\n{}",
            shelf[0].1
        );
        for (spelling, body) in &shelf[1..] {
            assert_eq!(
                *body, shelf[0].1,
                "`{spelling}` answered differently from `/agent` over the wire"
            );
        }

        // Bind and detach under each spelling, in turn, on the same thread.
        let legacy_add = dm_reply(
            &e2e,
            &member,
            &session,
            &mut sent,
            "/coach add @recovery-coach",
        )
        .await;
        assert_eq!(
            dm_conversation_coach(&e2e, &member).await.as_deref(),
            Some(installed_id.as_str()),
            "/coach add bound the Telegram thread"
        );
        let legacy_remove = dm_reply(&e2e, &member, &session, &mut sent, "/coach remove").await;
        assert_eq!(
            dm_conversation_coach(&e2e, &member).await,
            None,
            "/coach remove detached it again"
        );

        let canonical_add = dm_reply(
            &e2e,
            &member,
            &session,
            &mut sent,
            "/agent add @recovery-coach",
        )
        .await;
        assert_eq!(
            dm_conversation_coach(&e2e, &member).await.as_deref(),
            Some(installed_id.as_str())
        );
        let canonical_remove = dm_reply(&e2e, &member, &session, &mut sent, "/agent remove").await;
        assert_eq!(dm_conversation_coach(&e2e, &member).await, None);

        assert!(
            canonical_add.contains("Recovery Coach"),
            "the wire confirmation names the agent: {canonical_add}"
        );
        assert_eq!(
            legacy_add, canonical_add,
            "/coach add answered differently from /agent add over the wire"
        );
        assert_eq!(
            legacy_remove, canonical_remove,
            "/coach remove answered differently from /agent remove over the wire"
        );
        assert_eq!(
            e2e.llm.turn_calls.load(Ordering::SeqCst),
            turns_before,
            "no alias may fall through to the LLM"
        );
    }

    // ================================================================
    // The menu: canonical only
    // ================================================================

    /// Telegram's `/` menu publishes the canonical spelling and no alias of a
    /// command that can already publish its own name — so `agent` is offered
    /// and `coach`/`coaches` are not, even though both still work when typed.
    #[tokio::test]
    async fn the_telegram_menu_publishes_the_canonical_spelling_only() {
        let (registry, definitions) = catalogue();
        let menu = registry.bot_command_list();
        let names: Vec<&str> = menu.iter().map(|(name, _)| name.as_str()).collect();

        let agent = menu
            .iter()
            .find(|(name, _)| name == "agent")
            .expect("the shelf listing is in the menu under its canonical name");
        assert_eq!(
            agent.1,
            "List your installed agents — mention @handle for one turn, /agent add @handle to bind",
            "the menu row carries the catalogue's own description, canonical spelling included"
        );
        for absent in ["coach", "coaches"] {
            assert!(
                !names.contains(&absent),
                "/{absent} is a permanent alias, not a published menu row; menu was {names:?}"
            );
        }

        // The general rule the two above are instances of: an alias reaches
        // the menu ONLY as a stand-in for a multi-word trigger Telegram cannot
        // accept.
        for def in &definitions {
            if single_token(&def.command).is_none() {
                continue;
            }
            for alias in &def.aliases {
                if let Some(name) = single_token(alias) {
                    assert!(
                        !names.contains(&name.as_str()),
                        "`{alias}` is a synonym of the already-publishable {}; \
                         a menu of synonyms is harder to read than the one it padded",
                        def.command
                    );
                }
            }
        }
        // ...and the rule is not vacuous: /gs stands in for /group status,
        // which Telegram would reject.
        assert!(
            names.contains(&"gs"),
            "an alias of a multi-word command still reaches the menu: {names:?}"
        );
    }
}
