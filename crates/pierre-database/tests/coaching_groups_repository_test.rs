// ABOUTME: Covers the coaching-group repository against whichever backend DATABASE_URL names
// ABOUTME: Pins that every listing returns the same group the by-id read does, channel binding included
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The group listings had no direct repository test on either backend; the
//! route tests read groups by id, where both backends selected every
//! column. `create_test_db` opens whichever `DATABASE_URL` names, so the
//! same assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Utc;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupRespondMode, UpdateGroupRequest,
};
use pierre_core::models::{AgentCategory, CreateAgentRequest, Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// The user, tenant and agent rows a group's foreign keys resolve against.
async fn seed_owner_tenant_agent(repos: &RepositoryRegistry) -> (Uuid, TenantId, String) {
    let user = User::new(
        format!("group-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Group Owner".to_owned()),
    );
    let user_id = repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "Group Tenant".to_owned(),
        format!("group-tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user_id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    let agent = repos
        .agents
        .create(
            user_id,
            tenant.id,
            &CreateAgentRequest {
                title: "Group Coach".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
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
        .unwrap();
    (user_id, tenant.id, agent.id.to_string())
}

fn group_bound_to_telegram(tenant_id: TenantId, owner_id: Uuid, agent_id: &str) -> CoachingGroup {
    let now = Utc::now();
    CoachingGroup {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        name: "Marathon crew".to_owned(),
        description: Some("Spring marathon build".to_owned()),
        agent_id: agent_id.to_owned(),
        owner_id,
        coach_user_id: None,
        peer_data_sharing: true,
        respond_mode: GroupRespondMode::default(),
        // Not the default, so a listing that dropped the column would read
        // back a different value than the by-id read.
        digest_mode: GroupDigestMode::Chat,
        max_members: 12,
        is_active: true,
        channel_type: Some("telegram".to_owned()),
        channel_chat_id: Some("-100888555".to_owned()),
        created_at: now,
        updated_at: now,
    }
}

/// A group bound to a chat channel lists with that binding. `SQLite`'s
/// per-tenant and per-agent listings selected neither channel column, so
/// the tolerant row decoder handed back `None` for both while Postgres
/// returned the binding; the by-id read carried it on both.
#[tokio::test]
async fn listings_carry_the_channel_binding_the_by_id_read_carries() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (owner_id, tenant_id, agent_id) = seed_owner_tenant_agent(&repos).await;

    let created = repos
        .groups
        .create_group(
            tenant_id,
            &group_bound_to_telegram(tenant_id, owner_id, &agent_id),
        )
        .await
        .unwrap();
    assert_eq!(created.channel_type.as_deref(), Some("telegram"));
    assert_eq!(created.channel_chat_id.as_deref(), Some("-100888555"));

    let by_id = repos
        .groups
        .get_group(&created.id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("created group reads back");

    let for_tenant = repos
        .groups
        .list_active_groups_for_tenant(tenant_id)
        .await
        .unwrap();
    let listed = for_tenant
        .iter()
        .find(|g| g.id == created.id)
        .expect("the group is active in its tenant");
    assert_same_group(listed, &by_id, "the per-tenant listing");

    let for_agent = repos
        .groups
        .list_groups_for_agent(&agent_id, tenant_id)
        .await
        .unwrap();
    let listed = for_agent
        .iter()
        .find(|g| g.id == created.id)
        .expect("the group is listed under its agent");
    assert_same_group(listed, &by_id, "the per-agent listing");
    assert_eq!(by_id.digest_mode, GroupDigestMode::Chat);
}

/// Write a group row the way every row that predates the digest mode was
/// written: without the column, so the migration's default decides it.
async fn insert_group_without_digest_mode(
    db: &Database,
    id: Uuid,
    tenant_id: TenantId,
    owner_id: Uuid,
    agent_id: &str,
) {
    const SQL: &str = "INSERT INTO coaching_groups
         (id, tenant_id, name, agent_id, owner_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $6)";
    let now = Utc::now();
    match db {
        Database::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(id.to_string())
                .bind(tenant_id.to_string())
                .bind("Before the digest mode")
                .bind(agent_id)
                .bind(owner_id.to_string())
                .bind(now)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => {
            sqlx::query(SQL)
                .bind(id)
                .bind(tenant_id.to_string())
                .bind("Before the digest mode")
                .bind(agent_id)
                .bind(owner_id)
                .bind(now)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// Every group that existed before the digest mode reads as off — the
/// digest is opt-in — and the update writes a mode, which a later request
/// that does not name one leaves alone.
#[tokio::test]
async fn a_group_without_a_digest_mode_reads_off_until_an_update_sets_one() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (owner_id, tenant_id, agent_id) = seed_owner_tenant_agent(&repos).await;
    let id = Uuid::new_v4();
    insert_group_without_digest_mode(&db, id, tenant_id, owner_id, &agent_id).await;
    let group_id = id.to_string();

    let before = repos
        .groups
        .get_group(&group_id, tenant_id)
        .await
        .unwrap()
        .expect("the row reads back");
    assert_eq!(before.digest_mode, GroupDigestMode::Off);

    let set_managers = UpdateGroupRequest {
        name: None,
        description: None,
        agent_id: None,
        max_members: None,
        peer_data_sharing: None,
        respond_mode: None,
        digest_mode: Some(GroupDigestMode::Managers),
        is_active: None,
    };
    let updated = repos
        .groups
        .update_group(&group_id, tenant_id, &set_managers)
        .await
        .unwrap()
        .expect("the group is updated");
    assert_eq!(updated.digest_mode, GroupDigestMode::Managers);

    let rename = UpdateGroupRequest {
        name: Some("Renamed".to_owned()),
        digest_mode: None,
        ..set_managers
    };
    let renamed = repos
        .groups
        .update_group(&group_id, tenant_id, &rename)
        .await
        .unwrap()
        .expect("the group is updated");
    assert_eq!(renamed.name, "Renamed");
    assert_eq!(
        renamed.digest_mode,
        GroupDigestMode::Managers,
        "a request that does not name the mode keeps it"
    );
    let listed = repos
        .groups
        .list_active_groups_for_tenant(tenant_id)
        .await
        .unwrap();
    assert_eq!(
        listed.iter().find(|g| g.id == id).map(|g| g.digest_mode),
        Some(GroupDigestMode::Managers)
    );
}

/// Every column a listing decodes must match the by-id read of the same row.
fn assert_same_group(listed: &CoachingGroup, by_id: &CoachingGroup, listing: &str) {
    assert_eq!(
        listed.channel_type, by_id.channel_type,
        "{listing}: channel_type"
    );
    assert_eq!(
        listed.channel_chat_id, by_id.channel_chat_id,
        "{listing}: channel_chat_id"
    );
    assert_eq!(
        listed.respond_mode, by_id.respond_mode,
        "{listing}: respond_mode"
    );
    assert_eq!(
        listed.digest_mode, by_id.digest_mode,
        "{listing}: digest_mode"
    );
    assert_eq!(
        listed.coach_user_id, by_id.coach_user_id,
        "{listing}: coach_user_id"
    );
    assert_eq!(listed.name, by_id.name, "{listing}: name");
    assert_eq!(
        listed.description, by_id.description,
        "{listing}: description"
    );
    assert_eq!(listed.agent_id, by_id.agent_id, "{listing}: agent_id");
    assert_eq!(listed.owner_id, by_id.owner_id, "{listing}: owner_id");
    assert_eq!(listed.tenant_id, by_id.tenant_id, "{listing}: tenant_id");
    assert_eq!(
        listed.max_members, by_id.max_members,
        "{listing}: max_members"
    );
    assert_eq!(
        listed.peer_data_sharing, by_id.peer_data_sharing,
        "{listing}: peer_data_sharing"
    );
    assert_eq!(listed.is_active, by_id.is_active, "{listing}: is_active");
    assert_eq!(listed.created_at, by_id.created_at, "{listing}: created_at");
    assert_eq!(listed.updated_at, by_id.updated_at, "{listing}: updated_at");
}
