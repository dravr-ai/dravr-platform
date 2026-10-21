// ABOUTME: Team membership is an explicit choice: usage never joins the team, and a copy exists only when an athlete edits
// ABOUTME: Repository-level proofs of fork_agent, the membership writers and the owner-only edit, on whichever lane DATABASE_URL names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1096 / carnet#1098. "On the team" is an `agent_assignments` row
//! with `is_member`; usage only counts. A catalogue agent is never copied by
//! an install — a copy (a fork) is made when an athlete edits an agent they
//! cannot change, and it takes their membership, counters and selection
//! with it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers::agent_fixtures::publish_catalogue_agent;

use std::sync::Arc;

use pierre_core::errors::ErrorCode;
use pierre_core::field_update::FieldUpdate;
use pierre_core::models::agents::{
    AgentAssignment, AgentCategory, AgentHandle, AgentVisibility, CreateAgentRequest,
    ListAgentsFilter, UpdateAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Every runtime column an edit could leave unset, planted on the origin by
/// SQL so the fork has something to carry. Literals only: both lanes read them.
const PLANT_RUNTIME_COLUMNS: &str = "UPDATE agents SET \
     purpose = 'Pace the tempo runs.', when_to_use = 'Before a tempo session.', \
     instructions = 'You pace tempo runs.', example_inputs = 'How fast?', \
     example_outputs = 'Hold 4:10/km.', success_criteria = 'Even splits.', \
     startup_query = 'Show my last tempo run', \
     data_requirements = '{\"athlete_profile\":true}', \
     prerequisites = '{\"providers\":[\"strava\"],\"min_activities\":3}', \
     visuals = 'pace_chart,zones', max_tool_iterations = 7, temperature = 0.25, \
     sample_prompts = '[\"Plan my tempo\"]' \
     WHERE id = $1";

/// A French overlay on the origin, which the fork must carry.
const PLANT_TRANSLATION: &str = "INSERT INTO agent_translations \
     (agent_id, locale, title, description, purpose, instructions, tags, sample_prompts, created_at, updated_at) \
     VALUES ($1, 'fr', 'Coach Tempo', 'Allure seuil', 'Rythmer le seuil.', 'Tu rythmes.', '[]', '[]', \
     '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')";

const VISUALS_OF: &str = "SELECT visuals FROM agents WHERE id = $1";

async fn exec(database: &Database, sql: &str, agent_id: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(agent_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(agent_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

async fn visuals_of(database: &Database, agent_id: &str) -> Option<String> {
    match database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query_scalar(VISUALS_OF)
            .bind(agent_id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query_scalar(VISUALS_OF)
            .bind(agent_id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
    }
}

/// A fresh database and a user owning a tenant of their own.
async fn user(database: &Database, label: &str) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        database,
        &format!("{label}-{}@team.test", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    (user_id, tenant_id)
}

fn own_agent_request(title: &str) -> CreateAgentRequest {
    serde_json::from_value(serde_json::json!({
        "title": title,
        "system_prompt": format!("You are {title}."),
        "category": "training",
        "tags": ["own"],
    }))
    .unwrap()
}

async fn setup() -> (Arc<Database>, Arc<RepositoryRegistry>) {
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    (database, repos)
}

fn member_ids(assignments: &[AgentAssignment]) -> Vec<String> {
    assignments.iter().map(|a| a.user_id.clone()).collect()
}

// ============================================================================
// Usage is not membership
// ============================================================================

/// Opening a chat with a catalogue agent counts the use and nothing else: the
/// agent is in no team listing, `@handle` does not resolve it, and the Store
/// does not call it installed. Favouriting it, or activating it, is a choice
/// and does put it on the team; un-favouriting leaves the membership alone.
#[tokio::test]
async fn usage_never_joins_the_team_and_an_explicit_choice_does() {
    let (database, repos) = setup().await;
    let (author, author_tenant) = user(&database, "author").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin = publish_catalogue_agent(
        &repos,
        author,
        author_tenant,
        "Tempo Coach",
        "You are the tempo coach.",
    )
    .await;
    let id = origin.to_string();
    let handle = AgentHandle::parse("tempo-coach").unwrap();

    for _ in 0..2 {
        assert!(repos
            .agents
            .record_usage(&id, athlete, tenant)
            .await
            .unwrap());
    }
    assert!(repos
        .agents
        .list(athlete, tenant, &ListAgentsFilter::default())
        .await
        .unwrap()
        .iter()
        .all(|item| item.agent.id != origin));
    assert!(repos
        .agents
        .find_installed_by_handle(&handle, athlete, tenant)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .store_listings
        .get_installed_agents(athlete, tenant)
        .await
        .unwrap()
        .is_empty());
    assert!(repos.agents.list_assignments(&id).await.unwrap().is_empty());
    let state = repos
        .agents
        .get_user_preferences(&id, athlete)
        .await
        .unwrap();
    assert!(!state.is_member, "usage alone is not a membership");
    let (favorite, uses, last_used) = (state.is_favorite, state.use_count, state.last_used_at);
    assert!(!favorite);
    assert_eq!(uses, 2, "the uses are counted on the usage row");
    assert!(last_used.is_some());

    // Favouriting is a choice: the usage row becomes the membership.
    assert_eq!(
        repos
            .agents
            .toggle_favorite(&id, athlete, tenant)
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        member_ids(&repos.agents.list_assignments(&id).await.unwrap()),
        vec![athlete.to_string()]
    );
    let resolved = repos
        .agents
        .find_installed_by_handle(&handle, athlete, tenant)
        .await
        .unwrap()
        .expect("a favourite is on the team");
    assert_eq!(resolved.id, origin);
    let uses = repos
        .agents
        .get_user_preferences(&id, athlete)
        .await
        .unwrap()
        .use_count;
    assert_eq!(uses, 2, "joining keeps the counters");

    // Un-favouriting leaves them on the team.
    assert_eq!(
        repos
            .agents
            .toggle_favorite(&id, athlete, tenant)
            .await
            .unwrap(),
        Some(false)
    );
    assert_eq!(repos.agents.list_assignments(&id).await.unwrap().len(), 1);

    // Activating another catalogue agent is a choice as well.
    let other = publish_catalogue_agent(
        &repos,
        author,
        author_tenant,
        "Hill Coach",
        "You are the hill coach.",
    )
    .await;
    assert!(repos
        .agents
        .activate_agent(&other.to_string(), athlete, tenant)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        member_ids(
            &repos
                .agents
                .list_assignments(&other.to_string())
                .await
                .unwrap()
        ),
        vec![athlete.to_string()]
    );
}

/// An admin assignment promotes a usage row rather than adding a second one,
/// and taking the agent off the user's team keeps the counters.
#[tokio::test]
async fn an_assignment_promotes_the_usage_row_and_unassigning_keeps_its_counters() {
    let (database, repos) = setup().await;
    let (admin, admin_tenant) = user(&database, "admin").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin = publish_catalogue_agent(
        &repos,
        admin,
        admin_tenant,
        "Recovery Coach",
        "You recover.",
    )
    .await;
    let id = origin.to_string();

    for _ in 0..3 {
        assert!(repos
            .agents
            .record_usage(&id, athlete, tenant)
            .await
            .unwrap());
    }
    assert!(
        repos
            .agents
            .assign_agent(&id, athlete, admin)
            .await
            .unwrap(),
        "the usage row becomes a membership"
    );
    assert!(
        !repos
            .agents
            .assign_agent(&id, athlete, admin)
            .await
            .unwrap(),
        "already a member"
    );
    let members = repos.agents.list_assignments(&id).await.unwrap();
    assert_eq!(member_ids(&members), vec![athlete.to_string()]);
    assert_eq!(members[0].assigned_by.as_deref(), Some(&*admin.to_string()));

    assert!(repos.agents.unassign_agent(&id, athlete).await.unwrap());
    assert!(
        !repos.agents.unassign_agent(&id, athlete).await.unwrap(),
        "no membership left to end"
    );
    assert!(repos.agents.list_assignments(&id).await.unwrap().is_empty());
    let state = repos
        .agents
        .get_user_preferences(&id, athlete)
        .await
        .unwrap();
    let (uses, last_used) = (state.use_count, state.last_used_at);
    assert_eq!(uses, 3, "leaving the team keeps the usage counters");
    assert!(last_used.is_some());
}

/// A Store install of a catalogue agent writes no agents row, and a used
/// agent becomes a membership with its counters kept.
#[tokio::test]
async fn installing_a_catalogue_agent_writes_no_agent_row_and_promotes_its_usage() {
    let (database, repos) = setup().await;
    let (author, author_tenant) = user(&database, "author").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin =
        publish_catalogue_agent(&repos, author, author_tenant, "Fuel Coach", "You fuel.").await;
    let id = origin.to_string();
    assert!(repos
        .agents
        .record_usage(&id, athlete, tenant)
        .await
        .unwrap());
    let owned = repos.agents.count(athlete, tenant).await.unwrap();

    let installed = repos
        .store_listings
        .install_from_store(&id, athlete, tenant)
        .await
        .unwrap();
    assert_eq!(installed.id, origin);
    assert_eq!(repos.agents.count(athlete, tenant).await.unwrap(), owned);
    let uses = repos
        .agents
        .get_user_preferences(&id, athlete)
        .await
        .unwrap()
        .use_count;
    assert_eq!(uses, 1, "the usage row became the membership");
    assert_eq!(
        repos
            .store_listings
            .get_listing(&id)
            .await
            .unwrap()
            .unwrap()
            .install_count,
        1
    );

    let again = repos
        .store_listings
        .install_from_store(&id, athlete, tenant)
        .await
        .expect_err("a member is already installed");
    assert_eq!(again.code, ErrorCode::InvalidInput);
    assert_eq!(
        repos
            .store_listings
            .get_listing(&id)
            .await
            .unwrap()
            .unwrap()
            .install_count,
        1,
        "a refused install is not counted"
    );
}

// ============================================================================
// Fork on edit
// ============================================================================

/// The fork runs exactly as the origin did: every content and runtime column
/// and the translation overlays come along, the copy is private, not system,
/// `custom`, and carries the origin's handle. The athlete's row — membership,
/// counters, favourite — and their selection move onto it.
#[tokio::test]
async fn a_fork_copies_every_runtime_column_and_takes_the_membership_and_selection() {
    let (database, repos) = setup().await;
    let (author, author_tenant) = user(&database, "author").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin_uuid = publish_catalogue_agent(
        &repos,
        author,
        author_tenant,
        "Tempo Coach",
        "You are the tempo coach.",
    )
    .await;
    let origin_id = origin_uuid.to_string();
    exec(&database, PLANT_RUNTIME_COLUMNS, &origin_id).await;
    exec(&database, PLANT_TRANSLATION, &origin_id).await;
    let origin = repos
        .agents
        .get_by_id(&origin_id, athlete, tenant)
        .await
        .unwrap()
        .expect("a catalogue agent is visible to every athlete");
    assert!(origin.is_system);
    assert_eq!(origin.max_tool_iterations, Some(7), "the plant landed");

    repos
        .store_listings
        .install_from_store(&origin_id, athlete, tenant)
        .await
        .unwrap();
    for _ in 0..2 {
        repos
            .agents
            .record_usage(&origin_id, athlete, tenant)
            .await
            .unwrap();
    }
    repos
        .agents
        .toggle_favorite(&origin_id, athlete, tenant)
        .await
        .unwrap();
    repos
        .agents
        .activate_agent(&origin_id, athlete, tenant)
        .await
        .unwrap();

    let copy = repos
        .agents
        .fork_agent(&origin_id, athlete, tenant)
        .await
        .unwrap();
    let copy_id = copy.id.to_string();

    assert_ne!(copy.id, origin.id);
    assert_eq!(copy.forked_from, Some(origin.id));
    assert_eq!(copy.user_id, athlete);
    assert_eq!(copy.tenant_id, tenant.to_string());
    assert!(!copy.is_system);
    assert_eq!(copy.visibility, AgentVisibility::Private);
    assert_eq!(copy.source, "custom", "the athlete owns the text now");
    assert_eq!(copy.handle, origin.handle);
    assert_eq!(copy.handle.as_deref(), Some("tempo-coach"));

    assert_eq!(copy.title, origin.title);
    assert_eq!(copy.description, origin.description);
    assert_eq!(copy.system_prompt, origin.system_prompt);
    assert_eq!(copy.category, origin.category);
    assert_eq!(copy.tags, origin.tags);
    assert_eq!(copy.sample_prompts, origin.sample_prompts);
    assert_eq!(copy.token_count, origin.token_count);
    assert_eq!(copy.purpose, origin.purpose);
    assert_eq!(copy.when_to_use, origin.when_to_use);
    assert_eq!(copy.instructions, origin.instructions);
    assert_eq!(copy.example_inputs, origin.example_inputs);
    assert_eq!(copy.example_outputs, origin.example_outputs);
    assert_eq!(copy.success_criteria, origin.success_criteria);
    assert_eq!(copy.startup_query, origin.startup_query);
    assert_eq!(
        serde_json::to_value(&copy.data_requirements).unwrap(),
        serde_json::to_value(&origin.data_requirements).unwrap()
    );
    assert!(origin.data_requirements.is_some());
    assert_eq!(
        serde_json::to_value(&copy.prerequisites).unwrap(),
        serde_json::to_value(&origin.prerequisites).unwrap()
    );
    assert_eq!(copy.max_tool_iterations, origin.max_tool_iterations);
    assert_eq!(copy.temperature, origin.temperature);
    assert!(origin.temperature.is_some());
    assert_eq!(
        visuals_of(&database, &copy_id).await.as_deref(),
        Some("pace_chart,zones")
    );

    let mut translated = vec![copy.clone()];
    repos
        .agents
        .translate_agents(&mut translated, "fr")
        .await
        .unwrap();
    assert_eq!(
        translated[0].title, "Coach Tempo",
        "the overlays came along"
    );

    // The membership, counters and favourite moved; the origin holds no row.
    assert!(repos
        .agents
        .list_assignments(&origin_id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        member_ids(&repos.agents.list_assignments(&copy_id).await.unwrap()),
        vec![athlete.to_string()]
    );
    let team = repos
        .agents
        .list(athlete, tenant, &ListAgentsFilter::default())
        .await
        .unwrap();
    let item = team
        .iter()
        .find(|item| item.agent.id == copy.id)
        .expect("the copy is on the team");
    assert!(item.is_assigned);
    assert!(item.is_favorite);
    assert!(item.is_active, "the selection followed the fork");
    assert_eq!(item.use_count, 2);
    assert!(team.iter().all(|item| item.agent.id != origin.id));
    assert_eq!(
        repos
            .tenants
            .get_selected_agent(tenant, athlete)
            .await
            .unwrap()
            .as_deref(),
        Some(copy_id.as_str())
    );
    let installed: Vec<Uuid> = repos
        .store_listings
        .get_installed_agents(athlete, tenant)
        .await
        .unwrap()
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(installed, vec![copy.id], "the copy replaced the membership");
    assert_eq!(
        repos
            .store_listings
            .get_listing(&origin_id)
            .await
            .unwrap()
            .unwrap()
            .install_count,
        1,
        "a member's fork moves their install, it adds none"
    );
}

/// Forking twice returns the same copy; forking one's own agent returns it.
#[tokio::test]
async fn a_fork_is_made_once_and_an_own_agent_is_already_editable() {
    let (database, repos) = setup().await;
    let (author, author_tenant) = user(&database, "author").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin =
        publish_catalogue_agent(&repos, author, author_tenant, "Hill Coach", "You climb.").await;
    let origin_id = origin.to_string();

    let first = repos
        .agents
        .fork_agent(&origin_id, athlete, tenant)
        .await
        .unwrap();
    let second = repos
        .agents
        .fork_agent(&origin_id, athlete, tenant)
        .await
        .unwrap();
    assert_eq!(first.id, second.id, "one copy per origin and tenant");
    assert_eq!(repos.agents.count(athlete, tenant).await.unwrap(), 1);
    assert_eq!(
        member_ids(
            &repos
                .agents
                .list_assignments(&first.id.to_string())
                .await
                .unwrap()
        ),
        vec![athlete.to_string()],
        "an athlete who held no row gets a membership on the copy"
    );

    let of_copy = repos
        .agents
        .fork_agent(&first.id.to_string(), athlete, tenant)
        .await
        .unwrap();
    assert_eq!(of_copy.id, first.id, "a copy is already theirs to edit");

    let own = repos
        .agents
        .create(athlete, tenant, &own_agent_request("My Coach"))
        .await
        .unwrap();
    let forked_own = repos
        .agents
        .fork_agent(&own.id.to_string(), athlete, tenant)
        .await
        .unwrap();
    assert_eq!(forked_own.id, own.id);
    assert_eq!(forked_own.forked_from, None);
}

/// An agent the user cannot see is not forked, and neither is one they can
/// see but that their tenant cannot run.
#[tokio::test]
async fn a_fork_of_an_agent_the_user_cannot_reach_is_not_found() {
    let (database, repos) = setup().await;
    let (owner, owner_tenant) = user(&database, "owner").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let private = repos
        .agents
        .create(owner, owner_tenant, &own_agent_request("Private Coach"))
        .await
        .unwrap();
    let private_id = private.id.to_string();

    let err = repos
        .agents
        .fork_agent(&private_id, athlete, tenant)
        .await
        .expect_err("another user's private agent is invisible");
    assert_eq!(err.code, ErrorCode::ResourceNotFound);

    // Assigned to the athlete, so visible — but it belongs to another tenant.
    repos
        .agents
        .assign_agent(&private_id, athlete, owner)
        .await
        .unwrap();
    let err = repos
        .agents
        .fork_agent(&private_id, athlete, tenant)
        .await
        .expect_err("not reachable from the athlete's tenant");
    assert_eq!(err.code, ErrorCode::ResourceNotFound);
    assert_eq!(repos.agents.count(athlete, tenant).await.unwrap(), 0);
}

// ============================================================================
// Only the owner edits
// ============================================================================

/// An edit of an agent the caller can see but does not own is refused before
/// anything is written: no change, and no version row on the owner's history.
#[tokio::test]
async fn an_edit_of_an_agent_the_user_does_not_own_writes_nothing() {
    let (database, repos) = setup().await;
    let (author, author_tenant) = user(&database, "author").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin = publish_catalogue_agent(
        &repos,
        author,
        author_tenant,
        "Tempo Coach",
        "You are the tempo coach.",
    )
    .await;
    let origin_id = origin.to_string();
    repos
        .store_listings
        .install_from_store(&origin_id, athlete, tenant)
        .await
        .unwrap();
    let versions_before = repos
        .agents
        .get_versions(&origin_id, author_tenant, 50)
        .await
        .unwrap()
        .len();

    let edit = UpdateAgentRequest {
        title: Some("Hijacked".to_owned()),
        description: None,
        system_prompt: Some("You are someone else.".to_owned()),
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
    let refused = repos
        .agents
        .update(&origin_id, athlete, tenant, &edit, None)
        .await
        .unwrap();
    assert!(refused.is_none(), "the athlete does not own it");

    assert_eq!(
        repos
            .agents
            .get_versions(&origin_id, author_tenant, 50)
            .await
            .unwrap()
            .len(),
        versions_before,
        "a refused edit leaves no version behind"
    );
    let unchanged = repos
        .agents
        .get_by_id(&origin_id, athlete, tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.title, "Tempo Coach");
    assert_eq!(unchanged.category, AgentCategory::Training);

    // The way to edit it is a fork, which the athlete may change.
    let copy = repos
        .agents
        .fork_agent(&origin_id, athlete, tenant)
        .await
        .unwrap();
    let edited = repos
        .agents
        .update(&copy.id.to_string(), athlete, tenant, &edit, None)
        .await
        .unwrap()
        .expect("the athlete owns the fork");
    assert_eq!(edited.title, "Hijacked");
    let origin_after = repos
        .agents
        .get_by_id(&origin_id, athlete, tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(origin_after.title, "Tempo Coach", "the origin is untouched");
}

// ============================================================================
// Deleting an agent lets go of what ran on it
// ============================================================================

/// `chat_conversations.agent_id` has no `ON DELETE` rule, so a delete that
/// left a conversation on the agent failed outright. Deleting an edited copy
/// from the editor hands its conversation to the catalogue agent it was
/// forked from and gives that listing its install back; deleting an agent of
/// the athlete's own detaches its conversation.
#[tokio::test]
async fn deleting_an_agent_hands_its_conversations_on_instead_of_failing() {
    let (database, repos) = setup().await;
    let (author, author_tenant) = user(&database, "author").await;
    let (athlete, tenant) = user(&database, "athlete").await;
    let origin =
        publish_catalogue_agent(&repos, author, author_tenant, "Tempo Coach", "You pace.").await;
    let origin_id = origin.to_string();
    let athlete_key = athlete.to_string();

    let copy = repos
        .agents
        .fork_agent(&origin_id, athlete, tenant)
        .await
        .unwrap();
    let copy_id = copy.id.to_string();
    let on_copy = repos
        .chat
        .create_conversation(
            &athlete_key,
            tenant,
            "On the copy",
            "test-model",
            Some(&copy_id),
            None,
        )
        .await
        .unwrap()
        .id;
    assert!(repos
        .agents
        .delete(&copy_id, athlete, tenant)
        .await
        .unwrap());
    let moved = repos
        .chat
        .get_conversation(&on_copy, &athlete_key, tenant)
        .await
        .unwrap()
        .expect("the conversation survives the delete");
    assert_eq!(moved.agent_id.as_deref(), Some(origin_id.as_str()));

    let own = repos
        .agents
        .create(athlete, tenant, &own_agent_request("My Coach"))
        .await
        .unwrap();
    let own_id = own.id.to_string();
    let on_own = repos
        .chat
        .create_conversation(
            &athlete_key,
            tenant,
            "On my agent",
            "test-model",
            Some(&own_id),
            None,
        )
        .await
        .unwrap()
        .id;
    assert!(repos.agents.delete(&own_id, athlete, tenant).await.unwrap());
    let detached = repos
        .chat
        .get_conversation(&on_own, &athlete_key, tenant)
        .await
        .unwrap()
        .expect("the conversation survives the delete");
    assert_eq!(detached.agent_id, None);

    assert!(
        !repos
            .agents
            .delete(&origin_id, athlete, tenant)
            .await
            .unwrap(),
        "an agent the athlete does not own is not deleted"
    );
}
