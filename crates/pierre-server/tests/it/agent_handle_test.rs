// ABOUTME: Integration tests for the agent catalogue handle on SQLite
// ABOUTME: Approval assigns a unique @handle, and only agents on the athlete's team resolve it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_core::models::agents::{
    AgentCategory, AgentHandle, AgentVisibility, CreateSystemAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A second origin row claiming an owned handle, which `idx_agents_handle` must refuse.
const CLASH: &str =
    "INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, slug, created_at, updated_at) \
     VALUES ($1, $2, $3, 'Clash', 'prompt', 'tempo-coach', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')";

/// Create a system agent and take it through review to a published listing.
async fn publish_agent(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    tenant_id: TenantId,
    title: &str,
) -> Uuid {
    let agent = repos
        .agents
        .create_system_agent(
            author_id,
            tenant_id,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: format!("You are the {title}."),
                category: AgentCategory::Training,
                tags: vec!["test".to_owned()],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        agent.handle, None,
        "a coach owns no handle before it enters the catalogue"
    );

    let id = agent.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author_id, tenant_id)
        .await
        .unwrap();
    repos
        .store_listings
        .approve_agent(&id, tenant_id, Some(author_id))
        .await
        .unwrap();
    agent.id
}

#[test]
fn handle_derives_from_title_and_numbers_collisions() {
    let base = AgentHandle::derive("  Tempo & Threshold Coach! ");
    assert_eq!(base.as_str(), "tempo-threshold-coach");
    assert_eq!(base.candidate(0).as_str(), "tempo-threshold-coach");
    assert_eq!(base.candidate(1).as_str(), "tempo-threshold-coach-2");
    assert_eq!(base.candidate(9).as_str(), "tempo-threshold-coach-10");

    let long = AgentHandle::derive(&"x".repeat(60));
    assert_eq!(long.as_str().len(), AgentHandle::MAX_LEN);
    assert_eq!(long.candidate(1).as_str().len(), AgentHandle::MAX_LEN);
    assert!(long.candidate(1).as_str().ends_with("-2"));

    assert_eq!(AgentHandle::derive("!!!").as_str(), "coach");
}

#[test]
fn handle_parse_accepts_the_catalogue_alphabet_and_rejects_the_rest() {
    assert_eq!(
        AgentHandle::parse("@coach-tempo").unwrap().as_str(),
        "coach-tempo"
    );
    assert_eq!(
        AgentHandle::parse("strength_v2").unwrap().as_str(),
        "strength_v2"
    );
    for bad in [
        "",
        "-leading",
        "Upper",
        "with space",
        "acc/ent",
        &"a".repeat(41),
    ] {
        assert!(AgentHandle::parse(bad).is_err(), "{bad:?} must be rejected");
    }
}

#[tokio::test]
async fn approval_assigns_a_catalogue_unique_handle() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (author_id, _author, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, "author@handle.test", "starter")
            .await
            .unwrap();

    let first = publish_agent(repos, author_id, tenant_id, "Tempo Coach").await;
    let second = publish_agent(repos, author_id, tenant_id, "Tempo Coach").await;

    let first_listed = repos
        .store_listings
        .get_published_agent(&first.to_string())
        .await
        .unwrap()
        .unwrap();
    let second_listed = repos
        .store_listings
        .get_published_agent(&second.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_listed.agent.handle.as_deref(), Some("tempo-coach"));
    assert_eq!(second_listed.agent.handle.as_deref(), Some("tempo-coach-2"));

    // The unique index is the last line of defence: a second origin row
    // claiming an owned handle is refused by the database itself.
    let clash = match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query(CLASH)
            .bind(Uuid::new_v4().to_string())
            .bind(author_id.to_string())
            .bind(tenant_id)
            .execute(db.pool())
            .await
            .map(|_| ()),
        // `agents.user_id` and `tenant_id` are `uuid` columns on PostgreSQL.
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query(CLASH)
            .bind(Uuid::new_v4().to_string())
            .bind(author_id)
            .bind(tenant_id.as_uuid())
            .execute(db.pool())
            .await
            .map(|_| ()),
    };
    let err = clash.expect_err("duplicate origin handle must violate idx_agents_handle");
    assert!(
        err.to_string().to_lowercase().contains("unique"),
        "expected a UNIQUE violation, got: {err}"
    );
}

#[tokio::test]
async fn install_puts_the_origin_on_the_team_and_only_members_resolve_its_handle() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (author_id, _author, author_tenant) =
        create_test_user_with_plan(&resources.agent.database, "author@handle.test", "starter")
            .await
            .unwrap();
    let (athlete_id, _athlete, athlete_tenant) =
        create_test_user_with_plan(&resources.agent.database, "athlete@handle.test", "starter")
            .await
            .unwrap();
    let (bystander_id, _bystander, bystander_tenant) = create_test_user_with_plan(
        &resources.agent.database,
        "bystander@handle.test",
        "starter",
    )
    .await
    .unwrap();

    let origin = publish_agent(repos, author_id, author_tenant, "Recovery Coach").await;
    let handle = AgentHandle::parse("recovery-coach").unwrap();

    // Installing a catalogue agent across tenants puts the agent itself on
    // the athlete's team: no copy, the origin's own id and handle.
    let installed = repos
        .store_listings
        .install_from_store(&origin.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert_eq!(installed.id, origin, "a catalogue install is never a copy");
    assert_eq!(installed.forked_from, None);
    assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));

    let listed = repos
        .store_listings
        .get_installed_agents(athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, origin);
    assert_eq!(listed[0].handle.as_deref(), Some("recovery-coach"));

    // The athlete resolves the catalogue agent itself.
    let resolved = repos
        .agents
        .find_installed_by_handle(&handle, athlete_id, athlete_tenant)
        .await
        .unwrap()
        .expect("a coach on the team resolves by handle");
    assert_eq!(resolved.id, origin);
    assert_eq!(resolved.title, "Recovery Coach");

    // A user who never installed it gets nothing, even though the catalogue
    // lists the agent — and using it does not change that.
    assert!(repos
        .agents
        .record_usage(&origin.to_string(), bystander_id, bystander_tenant)
        .await
        .unwrap());
    let none = repos
        .agents
        .find_installed_by_handle(&handle, bystander_id, bystander_tenant)
        .await
        .unwrap();
    assert!(
        none.is_none(),
        "a coach the user only used is not on their team"
    );

    // Uninstalling — by the origin's id — withdraws the name.
    let source = repos
        .store_listings
        .uninstall_agent(&origin.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert_eq!(source, origin.to_string());
    let gone = repos
        .agents
        .find_installed_by_handle(&handle, athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert!(gone.is_none(), "an uninstalled coach must stop resolving");
}
