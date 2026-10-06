// ABOUTME: carnet#482 — /deleteaccount over the real Telegram webhook: warn, confirm by typed email, delete
// ABOUTME: Same refusals as the app (blockers, mismatch, rooms), and no row of the athlete survives the delete

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod delete_account {
    use std::env;
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::{TimeDelta, Utc};
    use pierre_contremaitre::messaging_strings::{
        DEFAULT_LOCALE, KEY_DELETE_ACCOUNT_DM_ONLY, KEY_DELETE_ACCOUNT_DONE,
        KEY_DELETE_ACCOUNT_EMAIL_MISMATCH,
    };
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupDigestMode, GroupRespondMode, GroupRole,
    };
    use pierre_database::backends::factory::DatabaseBackend;
    #[cfg(feature = "postgresql")]
    use pierre_database::repositories::POSTGRES_USER_PURGE;
    use pierre_database::repositories::SQLITE_USER_PURGE;
    use serial_test::serial;
    use uuid::Uuid;

    use crate::common::{
        create_test_server_resources_with_chat_provider,
        create_test_server_resources_with_chat_provider_and_config, test_server_config,
    };
    use crate::helpers::command_e2e::{CommandE2e, Member, RoomE2e, RouterLlm};
    use crate::helpers::identity_toolkit_stub::{Answer, IdentityToolkitStub, PROJECT};

    /// The account row, its tenant memberships and its channel links: the
    /// rows the account delete's foreign keys take along rather than clear
    /// themselves.
    const CASCADED_ROWS_SQL: &str = r"
        SELECT (SELECT COUNT(*) FROM users WHERE CAST(id AS TEXT) = $1)
             + (SELECT COUNT(*) FROM tenant_users WHERE CAST(user_id AS TEXT) = $1)
             + (SELECT COUNT(*) FROM messaging_channel_links WHERE CAST(user_id AS TEXT) = $1)
             + (SELECT COUNT(*) FROM messaging_sessions WHERE CAST(user_id AS TEXT) = $1)";

    async fn e2e() -> Arc<CommandE2e> {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        CommandE2e::start(resources, llm).await
    }

    /// A server whose Identity Toolkit is a stub answering every delete with
    /// `answer`.
    async fn e2e_with_firebase(answer: Answer) -> (Arc<CommandE2e>, Arc<IdentityToolkitStub>) {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let firebase = IdentityToolkitStub::answering(answer);
        let mut config = test_server_config();
        config.firebase = firebase.serve().await;
        let resources = create_test_server_resources_with_chat_provider_and_config(
            Arc::clone(&llm) as _,
            config,
        )
        .await
        .unwrap();
        (CommandE2e::start(resources, llm).await, firebase)
    }

    /// Give `member` the Firebase user a Google sign-in would have created.
    async fn link_firebase(e2e: &CommandE2e, member: &Member) -> String {
        let repos = &e2e.resources.common.repos;
        let mut user = repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .expect("the member exists");
        let uid = format!("firebase-{}", Uuid::new_v4().simple());
        user.firebase_uid = Some(uid.clone());
        repos.users.update(&user).await.unwrap();
        uid
    }

    async fn email_of(e2e: &CommandE2e, member: &Member) -> String {
        e2e.resources
            .common
            .repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .expect("the member exists")
            .email
    }

    /// Rows still held for `user_id`, by the engine's own purge statement plus
    /// the cascaded tables.
    async fn surviving(e2e: &CommandE2e, user_id: Uuid) -> (Vec<String>, i64) {
        let owner = user_id.to_string();
        match e2e.resources.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                let survivors: Vec<(String, i64)> = sqlx::query_as(SQLITE_USER_PURGE.surviving)
                    .bind(&owner)
                    .fetch_all(db.pool())
                    .await
                    .unwrap();
                let cascaded: i64 = sqlx::query_scalar(CASCADED_ROWS_SQL)
                    .bind(&owner)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
                (survivors.into_iter().map(|(t, _)| t).collect(), cascaded)
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                let survivors: Vec<(String, i64)> = sqlx::query_as(POSTGRES_USER_PURGE.surviving)
                    .bind(&owner)
                    .fetch_all(db.pool())
                    .await
                    .unwrap();
                let cascaded: i64 = sqlx::query_scalar(CASCADED_ROWS_SQL)
                    .bind(&owner)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
                (survivors.into_iter().map(|(t, _)| t).collect(), cascaded)
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_bare_command_warns_with_the_confirmation_and_deletes_nothing() {
        let e2e = e2e().await;
        let member = e2e.linked_member(false).await;
        let email = email_of(&e2e, &member).await;

        let ack = e2e.send_dm(&member, "/deleteaccount").await;
        assert_eq!(ack.messages_stored(), 0, "dispatched as a command");
        e2e.wait_outbound_containing(&format!("/deleteaccount {email}"), 1)
            .await;
        assert!(e2e
            .resources
            .common
            .repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_wrong_email_is_refused_and_the_account_stands() {
        let e2e = e2e().await;
        let member = e2e.linked_member(false).await;
        let mismatch = e2e
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_DELETE_ACCOUNT_EMAIL_MISMATCH, DEFAULT_LOCALE);

        e2e.send_dm(&member, "/deleteaccount someone-else@example.com")
            .await;
        // The reply is rich text: the egress renders its inline code, so the
        // ledger is matched on the sentence before it.
        let sentence = mismatch.split('`').next().unwrap().trim();
        e2e.wait_outbound_containing(sentence, 1).await;
        assert!(e2e
            .resources
            .common
            .repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_typed_email_deletes_the_account_and_leaves_no_row() {
        let e2e = e2e().await;
        let member = e2e.linked_member(true).await;
        let email = email_of(&e2e, &member).await;
        // A first turn forges the DM session, so the delete has one to clear.
        e2e.send_dm(&member, "/deleteaccount").await;
        e2e.wait_outbound_containing(&format!("/deleteaccount {email}"), 1)
            .await;
        let (before, cascaded_before) = surviving(&e2e, member.user_id).await;
        assert!(
            !before.is_empty() && cascaded_before >= 3,
            "the member must hold rows to delete: {before:?}, {cascaded_before}"
        );

        let ack = e2e
            .send_dm(&member, &format!("/deleteaccount {}", email.to_uppercase()))
            .await;
        assert_eq!(ack.messages_stored(), 0, "dispatched as a command");

        let repos = &e2e.resources.common.repos;
        assert!(
            repos
                .users
                .get_global(member.user_id)
                .await
                .unwrap()
                .is_none(),
            "the account is gone"
        );
        let (survivors, cascaded) = surviving(&e2e, member.user_id).await;
        assert!(
            survivors.is_empty(),
            "rows outlived the account: {survivors:?}"
        );
        assert_eq!(
            cascaded, 0,
            "an account, link, session or membership survived"
        );
        // The goodbye is delivered on a spawned turn after the delete. Let
        // it finish: delivering it must recreate nothing of the athlete's, and
        // it is not ledgered against the session the delete removed.
        e2e.resources
            .common
            .turns
            .drain(Duration::from_secs(30), Duration::from_secs(1))
            .await;
        let done = e2e
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_DELETE_ACCOUNT_DONE, DEFAULT_LOCALE);
        assert_eq!(e2e.outbound_count_containing(&done).await, 0);
        let (survivors, cascaded) = surviving(&e2e, member.user_id).await;
        assert!(survivors.is_empty(), "the goodbye left rows: {survivors:?}");
        assert_eq!(cascaded, 0, "the goodbye recreated a session or link");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_typed_email_deletes_the_firebase_identity_too() {
        let (e2e, firebase) = e2e_with_firebase(Answer::Deleted).await;
        let member = e2e.linked_member(false).await;
        let uid = link_firebase(&e2e, &member).await;
        let email = email_of(&e2e, &member).await;

        e2e.send_dm(&member, &format!("/deleteaccount {email}"))
            .await;

        assert!(e2e
            .resources
            .common
            .repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .is_none());
        let calls = firebase.calls();
        assert_eq!(calls.len(), 1, "one accounts:delete: {calls:?}");
        assert_eq!(calls[0].local_id, uid);
        assert_eq!(calls[0].project, PROJECT);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_firebase_outage_never_keeps_or_resurrects_the_account() {
        let (e2e, firebase) = e2e_with_firebase(Answer::Unavailable).await;
        let member = e2e.linked_member(false).await;
        let uid = link_firebase(&e2e, &member).await;
        let email = email_of(&e2e, &member).await;

        e2e.send_dm(&member, &format!("/deleteaccount {email}"))
            .await;

        let repos = &e2e.resources.common.repos;
        assert_eq!(firebase.calls().len(), 3, "a 503 is retried twice");
        assert!(repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .is_none());
        assert!(
            repos
                .users
                .get_by_firebase_uid(&uid)
                .await
                .unwrap()
                .is_none(),
            "nothing resurrected under the Firebase uid"
        );
        let (survivors, cascaded) = surviving(&e2e, member.user_id).await;
        assert!(survivors.is_empty(), "{survivors:?}");
        assert_eq!(cascaded, 0);
        let queued = repos
            .firebase_identity_deletions
            .due(Utc::now() + TimeDelta::days(365), 10)
            .await
            .unwrap();
        assert_eq!(queued.len(), 1, "the identity stays queued: {queued:?}");
        assert_eq!(queued[0].firebase_uid, uid);
        assert_eq!(queued[0].attempts, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_group_owner_is_refused_naming_the_group() {
        let e2e = e2e().await;
        let member = e2e.linked_member(false).await;
        let email = email_of(&e2e, &member).await;
        let repos = &e2e.resources.common.repos;
        let agent_id = repos
            .agents
            .create_system_agent(
                member.user_id,
                member.home_tenant,
                &CreateSystemAgentRequest {
                    title: "Hill coach".to_owned(),
                    description: None,
                    system_prompt: "You coach the group.".to_owned(),
                    category: AgentCategory::Recovery,
                    tags: vec![],
                    visibility: AgentVisibility::Tenant,
                    sample_prompts: vec![],
                },
            )
            .await
            .unwrap()
            .id
            .to_string();
        let now = Utc::now();
        repos
            .groups
            .create_group(
                member.home_tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: member.home_tenant.to_string(),
                    name: "Hill Thursdays".to_owned(),
                    description: None,
                    agent_id,
                    owner_id: member.user_id,
                    coach_user_id: None,
                    peer_data_sharing: false,
                    respond_mode: GroupRespondMode::default(),
                    digest_mode: GroupDigestMode::Off,
                    max_members: 10,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();

        e2e.send_dm(&member, &format!("/deleteaccount {email}"))
            .await;
        e2e.wait_outbound_containing("Hill Thursdays", 1).await;
        assert!(repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .is_some());
    }

    /// Typed in a shared room the confirmation would post the email to
    /// everyone in it: the command refuses there and the account stands.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_confirmation_is_refused_in_a_room() {
        let e2e = e2e().await;
        let owner = e2e.linked_member(false).await;
        let member = e2e.linked_member(false).await;
        let email = email_of(&e2e, &member).await;
        let room = RoomE2e::bind_room(
            Arc::clone(&e2e),
            -100_482_482,
            GroupRespondMode::default(),
            &owner,
        )
        .await;
        room.add_member(&member, GroupRole::Member).await;
        let dm_only = e2e
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_DELETE_ACCOUNT_DM_ONLY, DEFAULT_LOCALE);

        room.send_room_slash(&member, &format!("/deleteaccount {email}"))
            .await;
        let sentence = dm_only.split('`').next().unwrap().trim();
        e2e.wait_outbound_containing(sentence, 1).await;
        assert!(e2e
            .resources
            .common
            .repos
            .users
            .get_global(member.user_id)
            .await
            .unwrap()
            .is_some());
    }
}
