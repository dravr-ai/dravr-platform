// ABOUTME: PostgreSQL-lane tests for the unified conversation list and the per-participant read marker
// ABOUTME: Exercises the UUID/TIMESTAMPTZ bind paths the SQLite stand-in never touches

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` conversation-list tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::Utc;
use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{
    AddMessageParams, CoachingPersona, TenantId, User, UserStatus, UserTier,
};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

async fn seed_pg_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("list-{user_id}@test.local"),
        display_name: Some("Conversation List Test".to_owned()),
        password_hash: "hash_not_verified".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    db.repositories().users.create(&user).await.unwrap();
    user_id
}

/// Publish a catalogue coach (which assigns its `@handle`) and install it for
/// `athlete_id`, returning the installed copy's id.
async fn install_published_coach(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    author_tenant: TenantId,
    athlete_id: Uuid,
    athlete_tenant: TenantId,
) -> String {
    let coach = repos
        .coaches
        .create_system_coach(
            author_id,
            author_tenant,
            &CreateSystemCoachRequest {
                title: "Recovery Coach".to_owned(),
                description: Some("Rest-day specialist.".to_owned()),
                system_prompt: "You are the recovery coach.".to_owned(),
                category: CoachCategory::Training,
                tags: vec!["test".to_owned()],
                visibility: CoachVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    let id = coach.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author_id, author_tenant)
        .await
        .unwrap();
    repos
        .store_listings
        .approve_coach(&id, author_tenant, Some(author_id))
        .await
        .unwrap();
    repos
        .store_listings
        .install_from_store(&id, athlete_id, athlete_tenant)
        .await
        .unwrap()
        .id
        .to_string()
}

/// The unread count `user` sees on `conversation_id`, or `None` when the
/// thread is not in their list at all.
async fn unread_for(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    conversation_id: &str,
    user: &str,
) -> Option<i64> {
    repos
        .chat
        .list_conversations(user, tenant, 10, 0)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|c| c.id == conversation_id)
        .map(|c| c.unread_count)
}

async fn add_row(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    conversation_id: &str,
    user_id: Uuid,
    role: &str,
    content: &str,
) -> String {
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id,
            user_id: &user_id.to_string(),
            role,
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn test_pg_list_rows_carry_kind_facts_preview_paging_and_unread() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let author_id = seed_pg_user(&db).await;
    let author_tenant = TenantId::generate();
    let athlete_id = seed_pg_user(&db).await;
    let member_id = seed_pg_user(&db).await;
    let stranger_id = seed_pg_user(&db).await;
    let tenant = TenantId::generate();
    let athlete = athlete_id.to_string();
    let member = member_id.to_string();

    let coach_id =
        install_published_coach(&repos, author_id, author_tenant, athlete_id, tenant).await;

    let now = Utc::now();
    let group_id = Uuid::new_v4();
    repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: tenant.to_string(),
                name: "Marathon Squad".to_owned(),
                description: None,
                coach_id: coach_id.clone(),
                owner_id: athlete_id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id: athlete_id,
            tenant_id: tenant.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();

    // Three threads of three kinds, oldest first.
    let telegram = repos
        .chat
        .create_conversation(&athlete, tenant, "Messaging: telegram", "gpt-4", None, None)
        .await
        .unwrap();
    assert!(repos
        .chat
        .set_conversation_channel(&telegram.id, &athlete, tenant, "telegram")
        .await
        .unwrap());
    let grouped = repos
        .chat
        .create_conversation(
            &athlete,
            tenant,
            "Squad talk",
            "gpt-4",
            None,
            Some(&group_id.to_string()),
        )
        .await
        .unwrap();
    let coached = repos
        .chat
        .create_conversation(&athlete, tenant, "Recovery", "gpt-4", Some(&coach_id), None)
        .await
        .unwrap();
    let first = add_row(
        &repos,
        tenant,
        &coached.id,
        athlete_id,
        "user",
        "Comment va ma charge?",
    )
    .await;
    add_row(
        &repos,
        tenant,
        &coached.id,
        athlete_id,
        "tool_result",
        "<tool_result/>",
    )
    .await;
    let reply = "Ta charge grimpe.\n\n⟦viz:0⟧\n\nOn coupe jeudi.";
    add_row(&repos, tenant, &coached.id, athlete_id, "assistant", reply).await;

    // The page: every row fact, newest activity first, the real total.
    let page = repos
        .chat
        .list_conversations(&athlete, tenant, 10, 0)
        .await
        .unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(
        repos
            .chat
            .count_participating_conversations(&athlete, tenant)
            .await
            .unwrap(),
        3
    );
    let ids: Vec<&str> = page.items.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            coached.id.as_str(),
            grouped.id.as_str(),
            telegram.id.as_str()
        ]
    );

    let coach_row = &page.items[0];
    assert_eq!(coach_row.coach_id.as_deref(), Some(coach_id.as_str()));
    assert_eq!(coach_row.coach_handle.as_deref(), Some("recovery-coach"));
    assert_eq!(coach_row.coach_title.as_deref(), Some("Recovery Coach"));
    assert_eq!(coach_row.message_count, 2, "tool rows are not turns");
    assert_eq!(coach_row.unread_count, 2);
    let newest = coach_row.last_message.as_ref().expect("the newest row");
    assert_eq!(newest.role, "assistant");
    assert_eq!(
        newest.content_head, reply,
        "the head is the raw content; the route shapes it"
    );
    assert!(
        newest.created_at.contains('T'),
        "RFC 3339: {}",
        newest.created_at
    );
    assert!(coach_row.created_at.contains('T'));

    let group_row = &page.items[1];
    assert_eq!(
        group_row.group_id.as_deref(),
        Some(group_id.to_string().as_str())
    );
    assert_eq!(group_row.group_name.as_deref(), Some("Marathon Squad"));
    assert!(group_row.last_message.is_none());
    assert_eq!(group_row.unread_count, 0);

    assert_eq!(page.items[2].channel_type.as_deref(), Some("telegram"));

    // Paging is applied as given; the total is not.
    let page = repos
        .chat
        .list_conversations(&athlete, tenant, 2, 0)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 3);
    let page = repos
        .chat
        .list_conversations(&athlete, tenant, 2, 2)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, telegram.id);
    assert_eq!(page.total, 3);

    // The read marker: named row, newest row, monotonic, clear, refusals.
    assert!(repos
        .chat
        .mark_conversation_read(&coached.id, &athlete, tenant, Some(&first))
        .await
        .unwrap());
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &athlete).await,
        Some(1)
    );
    assert!(repos
        .chat
        .mark_conversation_read(&coached.id, &athlete, tenant, None)
        .await
        .unwrap());
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &athlete).await,
        Some(0)
    );
    assert!(repos
        .chat
        .mark_conversation_read(&coached.id, &athlete, tenant, Some(&first))
        .await
        .unwrap());
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &athlete).await,
        Some(0),
        "never backwards"
    );
    assert!(repos
        .chat
        .clear_conversation_read_marker(&coached.id, &athlete, tenant)
        .await
        .unwrap());
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &athlete).await,
        Some(2),
        "mark unread counts every turn"
    );
    assert!(!repos
        .chat
        .mark_conversation_read(&coached.id, &athlete, tenant, Some("not-a-row"))
        .await
        .unwrap());
    assert!(!repos
        .chat
        .mark_conversation_read(&coached.id, &stranger_id.to_string(), tenant, None)
        .await
        .unwrap());
    assert!(!repos
        .chat
        .clear_conversation_read_marker(&coached.id, &stranger_id.to_string(), tenant)
        .await
        .unwrap());

    // Membership: a stranger lists nothing; a member added later lists the
    // thread with their own marker, and what they post is new to the owner.
    assert_eq!(
        repos
            .chat
            .list_conversations(&stranger_id.to_string(), tenant, 10, 0)
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(unread_for(&repos, tenant, &coached.id, &member).await, None);
    repos
        .chat
        .add_participant(&coached.id, tenant, &member, &athlete)
        .await
        .unwrap();
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &member).await,
        Some(2)
    );
    assert!(repos
        .chat
        .mark_conversation_read(&coached.id, &athlete, tenant, None)
        .await
        .unwrap());
    add_row(
        &repos,
        tenant,
        &coached.id,
        member_id,
        "user",
        "three from the member",
    )
    .await;
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &athlete).await,
        Some(1)
    );
    assert_eq!(
        unread_for(&repos, tenant, &coached.id, &member).await,
        Some(3)
    );
    let member_page = repos
        .chat
        .list_conversations(&member, tenant, 10, 0)
        .await
        .unwrap();
    assert_eq!(member_page.total, 1);
    assert_eq!(
        member_page.items[0]
            .last_message
            .as_ref()
            .unwrap()
            .content_head,
        "three from the member"
    );
}
