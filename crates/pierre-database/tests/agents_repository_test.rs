// ABOUTME: Covers the agents repository, written once for both backends, against whichever backend DATABASE_URL names
// ABOUTME: Pins the reads where the two backends' copies used to disagree: temperature, hidden agents, search, list state

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The agents catalogue used to carry its SQL twice, and the copies had
//! drifted. Postgres decoded `agents.temperature` (`DOUBLE PRECISION`) as an
//! `f32`, so an agent's temperature read back as absent and the runtime
//! context of an agent with one set failed; `SQLite`'s hidden-agent listing
//! selected a narrower column list, so a hidden agent came back without its
//! startup query. One body now serves both, and `create_test_db` opens
//! whichever `DATABASE_URL` names, so these assertions run on `SQLite` and on
//! `PostgreSQL` alike.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::field_update::FieldUpdate;
use pierre_core::models::agents::{ListAgentsFilter, UpdateAgentRequest};
use pierre_core::models::{AgentCategory, CreateAgentRequest, Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

async fn seed_user(repos: &RepositoryRegistry, label: &str) -> Uuid {
    let user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some(label.to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

async fn seed_tenant(repos: &RepositoryRegistry, owner: Uuid) -> TenantId {
    let tenant = Tenant::new(
        "Agents Tenant".to_owned(),
        format!("agents-tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        owner,
    );
    repos.tenants.create(&tenant).await.unwrap();
    tenant.id
}

fn request(title: &str, category: AgentCategory) -> CreateAgentRequest {
    CreateAgentRequest {
        title: title.to_owned(),
        description: Some("A patient running coach".to_owned()),
        system_prompt: "You are helpful".to_owned(),
        category,
        tags: vec!["running".to_owned()],
        sample_prompts: vec!["How was my week?".to_owned()],
        startup_query: Some("Summarise my last run".to_owned()),
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: Some(7),
    }
}

/// Set a temperature the way the agent seeder writes one; no repository
/// method writes it.
async fn set_temperature(db: &Database, agent_id: &str, temperature: f64) {
    const SQL: &str = "UPDATE agents SET temperature = $1 WHERE id = $2";
    match db {
        Database::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(temperature)
                .bind(agent_id)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => {
            sqlx::query(SQL)
                .bind(temperature)
                .bind(agent_id)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// A stored temperature reads back on the agent and in its runtime context.
#[tokio::test]
async fn a_stored_temperature_reads_back() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = seed_user(&repos, "temperature").await;
    let tenant = seed_tenant(&repos, owner).await;
    let agent = repos
        .agents
        .create(owner, tenant, &request("Tempo", AgentCategory::Training))
        .await
        .unwrap();
    let agent_id = agent.id.to_string();
    set_temperature(&db, &agent_id, 0.25).await;

    let read = repos
        .agents
        .get_by_id(&agent_id, owner, tenant)
        .await
        .unwrap()
        .expect("the agent reads back");
    assert_eq!(read.temperature, Some(0.25));
    assert_eq!(read.max_tool_iterations, Some(7));
    assert_eq!(read.startup_query.as_deref(), Some("Summarise my last run"));
    assert_eq!(read.tags, vec!["running".to_owned()]);

    let context = repos
        .agents
        .get_agent_runtime_context(&agent_id, tenant)
        .await
        .unwrap()
        .expect("the runtime context reads back");
    assert_eq!(context.temperature, Some(0.25));
    assert_eq!(context.title, "Tempo");
    assert_eq!(context.max_tool_iterations, Some(7));
}

/// A hidden agent lists with every column the by-id read carries.
#[tokio::test]
async fn a_hidden_agent_lists_with_its_startup_query() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = seed_user(&repos, "hidden-owner").await;
    let athlete = seed_user(&repos, "hidden-athlete").await;
    let tenant = seed_tenant(&repos, owner).await;
    let agent = repos
        .agents
        .create(owner, tenant, &request("Hideable", AgentCategory::Recovery))
        .await
        .unwrap();
    let agent_id = agent.id.to_string();
    assert!(repos
        .agents
        .assign_agent(&agent_id, athlete, owner)
        .await
        .unwrap());
    assert!(repos
        .agents
        .hide_agent(&agent_id, athlete, tenant)
        .await
        .unwrap());

    let hidden = repos
        .agents
        .list_hidden_agents(athlete, tenant)
        .await
        .unwrap();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].id, agent.id);
    assert_eq!(
        hidden[0].startup_query.as_deref(),
        Some("Summarise my last run")
    );
    assert_eq!(hidden[0].max_tool_iterations, Some(7));

    let assignments = repos.agents.list_assignments(&agent_id).await.unwrap();
    let athlete_row = assignments
        .iter()
        .find(|a| a.user_id == athlete.to_string())
        .expect("the athlete's assignment lists");
    assert_eq!(athlete_row.assigned_by, Some(owner.to_string()));
    assert!(chrono::DateTime::parse_from_rfc3339(&athlete_row.assigned_at).is_ok());

    // Hidden agents drop out of the default listing and come back with the filter.
    let defaults = repos
        .agents
        .list(athlete, tenant, &ListAgentsFilter::with_defaults())
        .await
        .unwrap();
    assert!(defaults.iter().all(|item| item.agent.id != agent.id));
    let with_hidden = ListAgentsFilter {
        include_hidden: true,
        ..ListAgentsFilter::with_defaults()
    };
    let all = repos
        .agents
        .list(athlete, tenant, &with_hidden)
        .await
        .unwrap();
    assert!(all.iter().any(|item| item.agent.id == agent.id));

    assert!(repos.agents.show_agent(&agent_id, athlete).await.unwrap());
    assert!(repos
        .agents
        .list_hidden_agents(athlete, tenant)
        .await
        .unwrap()
        .is_empty());
}

/// The listing reports the user's own state on each agent, and every filter
/// narrows it.
#[tokio::test]
async fn the_listing_carries_the_users_state_and_honours_its_filters() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = seed_user(&repos, "listing").await;
    let tenant = seed_tenant(&repos, owner).await;
    let training = repos
        .agents
        .create(
            owner,
            tenant,
            &request("Intervals", AgentCategory::Training),
        )
        .await
        .unwrap();
    let nutrition = repos
        .agents
        .create(owner, tenant, &request("Fuel", AgentCategory::Nutrition))
        .await
        .unwrap();
    let training_id = training.id.to_string();

    assert!(repos
        .agents
        .record_usage(&training_id, owner, tenant)
        .await
        .unwrap());
    assert!(repos
        .agents
        .record_usage(&training_id, owner, tenant)
        .await
        .unwrap());
    assert_eq!(
        repos
            .agents
            .toggle_favorite(&training_id, owner, tenant)
            .await
            .unwrap(),
        Some(true)
    );
    assert!(repos
        .agents
        .activate_agent(&training_id, owner, tenant)
        .await
        .unwrap()
        .is_some());

    let items = repos
        .agents
        .list(owner, tenant, &ListAgentsFilter::with_defaults())
        .await
        .unwrap();
    let item = items
        .iter()
        .find(|i| i.agent.id == training.id)
        .expect("the used agent lists");
    assert!(item.is_assigned);
    assert!(item.is_favorite);
    assert!(item.is_active);
    assert_eq!(item.use_count, 2);
    assert!(item.last_used_at.is_some());
    let other = items
        .iter()
        .find(|i| i.agent.id == nutrition.id)
        .expect("the other agent lists");
    assert!(!other.is_favorite);
    assert!(!other.is_active);
    assert_eq!(other.use_count, 0);

    let favorites = ListAgentsFilter {
        favorites_only: true,
        ..ListAgentsFilter::with_defaults()
    };
    let favorite_ids: Vec<Uuid> = repos
        .agents
        .list(owner, tenant, &favorites)
        .await
        .unwrap()
        .iter()
        .map(|i| i.agent.id)
        .collect();
    assert_eq!(favorite_ids, vec![training.id]);

    let nutrition_only = ListAgentsFilter {
        category: Some(AgentCategory::Nutrition),
        ..ListAgentsFilter::with_defaults()
    };
    let nutrition_ids: Vec<Uuid> = repos
        .agents
        .list(owner, tenant, &nutrition_only)
        .await
        .unwrap()
        .iter()
        .map(|i| i.agent.id)
        .collect();
    assert_eq!(nutrition_ids, vec![nutrition.id]);

    let (favorite, uses, last_used) = repos
        .agents
        .get_user_preferences(&training_id, owner)
        .await
        .unwrap();
    assert!(favorite);
    assert_eq!(uses, 2);
    assert!(last_used.is_some());
    assert_eq!(repos.agents.count(owner, tenant).await.unwrap(), 2);
}

/// Search folds case on both backends.
#[tokio::test]
async fn search_ignores_case() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = seed_user(&repos, "search").await;
    let tenant = seed_tenant(&repos, owner).await;
    let agent = repos
        .agents
        .create(
            owner,
            tenant,
            &request("Marathon Builder", AgentCategory::Training),
        )
        .await
        .unwrap();

    let found = repos
        .agents
        .search(owner, tenant, "MARATHON", None, None)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, agent.id);
    assert!(repos
        .agents
        .search(owner, tenant, "triathlon", None, None)
        .await
        .unwrap()
        .is_empty());
}

/// An edit snapshots the prior state, and a revert restores it as a new version.
#[tokio::test]
async fn an_edit_is_versioned_and_a_revert_restores_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = seed_user(&repos, "versions").await;
    let tenant = seed_tenant(&repos, owner).await;
    let agent = repos
        .agents
        .create(owner, tenant, &request("Original", AgentCategory::Training))
        .await
        .unwrap();
    let agent_id = agent.id.to_string();

    let update = UpdateAgentRequest {
        title: Some("Renamed".to_owned()),
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Keep,
    };
    let updated = repos
        .agents
        .update(&agent_id, owner, tenant, &update, Some("rename"))
        .await
        .unwrap()
        .expect("the owner updates the agent");
    assert_eq!(updated.title, "Renamed");
    assert_eq!(
        updated.startup_query.as_deref(),
        Some("Summarise my last run"),
        "an update that does not name the startup query keeps it"
    );
    assert_eq!(
        repos.agents.get_current_version(&agent_id).await.unwrap(),
        1
    );

    let versions = repos
        .agents
        .get_versions(&agent_id, tenant, 10)
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[0].change_summary.as_deref(), Some("rename"));
    assert_eq!(versions[0].created_by, Some(owner));
    assert_eq!(versions[0].content_snapshot["title"], "Original");

    let reverted = repos
        .agents
        .revert_to_version(&agent_id, 1, owner, tenant)
        .await
        .unwrap();
    assert_eq!(reverted.title, "Original");
    assert_eq!(
        repos.agents.get_current_version(&agent_id).await.unwrap(),
        2
    );
}
