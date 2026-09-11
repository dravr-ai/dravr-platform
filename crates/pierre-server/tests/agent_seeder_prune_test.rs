// ABOUTME: The agent seeder deletes catalogue-owned agents whose markdown directory is gone
// ABOUTME: A merged agent hands its conversations, groups, installs and slug-keyed plans to its successor
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::fs;
use std::path::Path;

use chrono::Utc;
use pierre_core::models::groups::GroupRespondMode;
use pierre_core::models::{
    AgentCategory, AgentVisibility, CoachingGroup, CreateAgentRequest, CreateSystemAgentRequest,
    TenantId,
};
use pierre_database::repositories::training_plans::{PlanOwner, SaveTrainingPlanParams};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_seeders::agents::{self, SeedArgs};
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use tempfile::TempDir;
use uuid::Uuid;

const KEPT: &str = "kept-coach";
const RETIRED: &str = "retired-coach";
const BROKEN: &str = "broken-coach";

/// Bootstrap an operator so the agent seeder has an admin to own the rows.
async fn seeded_repos() -> (RepositoryRegistry, Uuid, TenantId) {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    bootstrap::run(
        BootstrapArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();
    let admin = repos
        .seeder
        .seed_get_admin_user()
        .await
        .unwrap()
        .expect("bootstrap seeds an admin");
    let tenant = repos
        .seeder
        .seed_get_user_tenant(admin.id)
        .await
        .unwrap()
        .expect("the admin has a tenant");
    (repos, admin.id, TenantId::parse_str(&tenant).unwrap())
}

/// Write one canonical `en.md` in the `<category>/<slug>/` layout the seeder
/// scans; `replaces` declares the retired agents this one absorbs.
fn write_agent(checkout: &Path, slug: &str, replaces: Option<&str>) {
    let dir = checkout.join("mobility").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let replaces = replaces.map_or_else(String::new, |r| format!("replaces: [{r}]\n"));
    fs::write(
        dir.join("en.md"),
        format!(
            "---\nname: {slug}\ntitle: {slug} title\ncategory: mobility\ntags: [prune]\n\
             prerequisites:\n  providers: []\n  min_activities: 0\n  activity_types: []\n\
             visibility: tenant\n{replaces}---\n\n## Purpose\nA coach that exercises the prune pass.\n\n\
             ## Instructions\nYou are {slug}. Say so.\n"
        ),
    )
    .unwrap();
}

fn remove_agent(checkout: &Path, slug: &str) {
    fs::remove_dir_all(checkout.join("mobility").join(slug)).unwrap();
}

async fn seed(repos: &RepositoryRegistry, checkout: &Path, dry_run: bool) -> bool {
    agents::run(
        SeedArgs {
            agents_dir: checkout.to_path_buf(),
            dry_run,
        },
        repos,
    )
    .await
    .is_ok()
}

async fn agent_id(repos: &RepositoryRegistry, slug: &str, tenant: TenantId) -> Option<String> {
    repos
        .seeder
        .seed_find_agent_by_slug(slug, &tenant.to_string())
        .await
        .unwrap()
        .map(|(id, _)| id)
}

async fn conversation_bound_to(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    agent: &str,
) -> String {
    repos
        .chat
        .create_conversation(
            &user.to_string(),
            tenant,
            "Prune",
            "test-model",
            Some(agent),
            None,
        )
        .await
        .unwrap()
        .id
}

async fn conversation_agent(
    repos: &RepositoryRegistry,
    conversation_id: &str,
    user: Uuid,
    tenant: TenantId,
) -> Option<String> {
    repos
        .chat
        .get_conversation(conversation_id, &user.to_string(), tenant)
        .await
        .unwrap()
        .expect("the conversation survives the prune")
        .agent_id
}

async fn group_bound_to(
    repos: &RepositoryRegistry,
    owner: Uuid,
    tenant: TenantId,
    agent: &str,
) -> Uuid {
    let now = Utc::now();
    repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant.to_string(),
                name: "Prune group".to_owned(),
                description: None,
                agent_id: agent.to_owned(),
                owner_id: owner,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id
}

async fn group_agent(repos: &RepositoryRegistry, group_id: Uuid, tenant: TenantId) -> String {
    repos
        .groups
        .get_group(&group_id.to_string(), tenant)
        .await
        .unwrap()
        .expect("the group survives the prune")
        .agent_id
}

/// A goal race is required to save an outline; its content is irrelevant here.
fn goal_race() -> GoalRace {
    GoalRace {
        name: "Prune Classic".to_owned(),
        date: "2027-05-01".to_owned(),
        discipline: "trail run".to_owned(),
        priority: RacePriority::A,
    }
}

/// Save an active outline owned by `slug` — a row keyed by the agent's *slug*,
/// not its id, which is what makes it invisible to the id-keyed hand-over.
async fn plan_owned_by(repos: &RepositoryRegistry, user: Uuid, tenant: TenantId, slug: &str) {
    let race = goal_race();
    repos
        .training_plans
        .save_training_plan(&SaveTrainingPlanParams {
            tenant_id: &tenant.to_string(),
            user_id: &user.to_string(),
            owner: PlanOwner::agent(slug),
            goal_fact_id: None,
            goal_race: &race,
            races: Some(&[]),
            strategy: "Build, then taper.",
            flavour: None,
            season_start: None,
            season_end: None,
            phases: &[],
            source_conversation_id: None,
        })
        .await
        .unwrap();
}

/// The athlete's active outline for `slug`, by the strategy it carries.
async fn plan_strategy(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    slug: &str,
) -> Option<String> {
    repos
        .training_plans
        .get_active_plan(
            &tenant.to_string(),
            &user.to_string(),
            PlanOwner::agent(slug),
        )
        .await
        .unwrap()
        .map(|plan| plan.strategy)
}

/// The successor's published install counter, as the store screen prints it.
async fn listing_install_count(repos: &RepositoryRegistry, agent: &str) -> u32 {
    repos
        .store_listings
        .get_listing(agent)
        .await
        .unwrap()
        .expect("the catalogue publishes a listing for every agent")
        .install_count
}

/// How many of `agent`'s installs belong to `user`.
async fn installs_for(repos: &RepositoryRegistry, agent: &str, user: Uuid) -> usize {
    repos
        .agents
        .list_assignments(agent)
        .await
        .unwrap()
        .iter()
        .filter(|a| a.user_id == user.to_string())
        .count()
}

/// The catalogue is the whole roster: once an agent's directory is gone from the
/// checkout, the next seed deletes its row, and the store listing goes with it.
/// The surviving agent keeps the very same row.
#[tokio::test]
async fn an_agent_whose_directory_is_gone_is_deleted_on_the_next_seed() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);

    let kept = agent_id(&repos, KEPT, tenant)
        .await
        .expect("the kept coach is seeded");
    let retired = agent_id(&repos, RETIRED, tenant)
        .await
        .expect("the retired coach is seeded while its file exists");
    assert!(
        repos
            .store_listings
            .get_listing(&retired)
            .await
            .unwrap()
            .is_some(),
        "a seeded coach is published to the store"
    );

    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(
        agent_id(&repos, RETIRED, tenant).await,
        None,
        "the retired coach is deleted once its directory is gone"
    );
    assert!(
        repos
            .store_listings
            .get_listing(&retired)
            .await
            .unwrap()
            .is_none(),
        "its store listing is gone too"
    );
    assert_eq!(
        agent_id(&repos, KEPT, tenant).await.as_deref(),
        Some(kept.as_str()),
        "the surviving coach keeps its row"
    );
}

/// Dry-run names what it would delete and touches nothing.
#[tokio::test]
async fn dry_run_reports_the_retirement_without_deleting() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);

    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), true).await);

    assert!(
        agent_id(&repos, RETIRED, tenant).await.is_some(),
        "dry-run leaves the retired coach in place"
    );
}

/// Only catalogue-owned rows are candidates. A system agent the operator wrote
/// in the console and an athlete's own agent share the tenant and are not in
/// any checkout, so they must survive every seed.
#[tokio::test]
async fn agents_that_never_came_from_the_catalogue_survive_the_prune() {
    let (repos, admin, tenant) = seeded_repos().await;
    let operator_coach = repos
        .agents
        .create_system_agent(
            admin,
            tenant,
            &CreateSystemAgentRequest {
                title: "Console coach".to_owned(),
                description: None,
                system_prompt: "You were written in the admin console.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap();
    let own_coach = repos
        .agents
        .create(
            admin,
            tenant,
            &CreateAgentRequest {
                title: "My coach".to_owned(),
                description: None,
                system_prompt: "You are an athlete's own coach.".to_owned(),
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
        .unwrap();

    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    assert!(seed(&repos, checkout.path(), false).await);

    assert!(
        repos
            .agents
            .get_system_agent(&operator_coach.id.to_string(), tenant)
            .await
            .unwrap()
            .is_some(),
        "an operator-authored system agent is not catalogue-owned"
    );
    assert!(
        repos
            .agents
            .get_by_id(&own_coach.id.to_string(), admin, tenant)
            .await
            .unwrap()
            .is_some(),
        "an athlete's own coach is not catalogue-owned"
    );
    assert!(
        agent_id(&repos, KEPT, tenant).await.is_some(),
        "the catalogue coach is seeded alongside them"
    );
}

/// An empty checkout is a broken clone, not an empty roster: the seeder stops
/// before any pass runs, so nothing is pruned.
#[tokio::test]
async fn an_empty_checkout_prunes_nothing() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    assert!(seed(&repos, checkout.path(), false).await);

    let empty = TempDir::new().unwrap();
    assert!(seed(&repos, empty.path(), false).await);

    assert!(
        agent_id(&repos, KEPT, tenant).await.is_some(),
        "a checkout with no coach files must not empty the roster"
    );
}

/// A merge is declared by the survivor: `replaces: [retired-agent]`.
///
/// The athlete's conversation and the group bound to the retired agent
/// continue with the successor, and only then does the retired row go.
#[tokio::test]
async fn a_merged_agent_hands_its_conversation_and_group_to_its_successor() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let kept = agent_id(&repos, KEPT, tenant).await.unwrap();
    let retired = agent_id(&repos, RETIRED, tenant).await.unwrap();
    let conversation = conversation_bound_to(&repos, admin, tenant, &retired).await;
    let group = group_bound_to(&repos, admin, tenant, &retired).await;

    write_agent(checkout.path(), KEPT, Some(RETIRED));
    remove_agent(checkout.path(), RETIRED);
    assert!(
        seed(&repos, checkout.path(), false).await,
        "the seed succeeds once the references are handed over"
    );

    assert_eq!(agent_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        conversation_agent(&repos, &conversation, admin, tenant)
            .await
            .as_deref(),
        Some(kept.as_str()),
        "the conversation continues with the successor"
    );
    assert_eq!(
        group_agent(&repos, group, tenant).await,
        kept,
        "the group continues with the successor"
    );
}

/// With no successor declared, a conversation is detached and drops to the
/// default prompt rather than blocking the delete.
#[tokio::test]
async fn a_deleted_agent_with_no_successor_detaches_its_conversation() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let retired = agent_id(&repos, RETIRED, tenant).await.unwrap();
    let conversation = conversation_bound_to(&repos, admin, tenant, &retired).await;

    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(agent_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        conversation_agent(&repos, &conversation, admin, tenant).await,
        None,
        "the conversation is detached, not deleted"
    );
}

/// A group needs an agent, so one bound to a retired agent with no successor blocks the delete.
///
/// The seed reports the failure and the row stays, so an operator picks a
/// agent for the group instead of the seeder guessing.
#[tokio::test]
async fn a_group_bound_to_an_agent_with_no_successor_blocks_the_delete() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let retired = agent_id(&repos, RETIRED, tenant).await.unwrap();
    let group = group_bound_to(&repos, admin, tenant, &retired).await;

    remove_agent(checkout.path(), RETIRED);
    assert!(
        !seed(&repos, checkout.path(), false).await,
        "the seed reports the blocked delete"
    );

    assert_eq!(
        agent_id(&repos, RETIRED, tenant).await.as_deref(),
        Some(retired.as_str()),
        "the retired coach stays while a group needs it"
    );
    assert_eq!(group_agent(&repos, group, tenant).await, retired);
}

/// A refused agent file is an agent the seeder cannot see, not one that left the catalogue.
///
/// The prune pass is suspended for that run.
#[tokio::test]
async fn an_agent_file_that_fails_to_parse_suspends_the_prune() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    write_agent(checkout.path(), BROKEN, None);
    assert!(seed(&repos, checkout.path(), false).await);

    fs::write(
        checkout.path().join("mobility").join(BROKEN).join("en.md"),
        "---\nname: [not: valid\n---\n",
    )
    .unwrap();
    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert!(
        agent_id(&repos, RETIRED, tenant).await.is_some(),
        "nothing is pruned while a file fails to parse"
    );
    assert!(
        agent_id(&repos, BROKEN, tenant).await.is_some(),
        "the unparseable coach keeps its row"
    );
}

/// An install is a row in `agent_assignments`, which `ON DELETE CASCADE`s with
/// the agent — so without the hand-over the athlete's install is *deleted*,
/// not merely orphaned, and they silently lose the agent they chose.
#[tokio::test]
async fn a_merged_agent_hands_its_install_to_its_successor() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let kept = agent_id(&repos, KEPT, tenant).await.unwrap();
    let retired = agent_id(&repos, RETIRED, tenant).await.unwrap();
    repos
        .agents
        .assign_agent(&retired, admin, admin)
        .await
        .unwrap();
    assert_eq!(installs_for(&repos, &retired, admin).await, 1);

    write_agent(checkout.path(), KEPT, Some(RETIRED));
    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(agent_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        installs_for(&repos, &kept, admin).await,
        1,
        "the athlete keeps the agent they installed, under its successor"
    );
}

/// An athlete holding both agents already has the successor's row, and
/// `agent_assignments` is `UNIQUE(agent_id, user_id)` — so a blind `UPDATE`
/// would collide and fail the whole seed. They end with one install, not two,
/// and not an error.
#[tokio::test]
async fn an_athlete_holding_both_agents_keeps_a_single_install() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let kept = agent_id(&repos, KEPT, tenant).await.unwrap();
    let retired = agent_id(&repos, RETIRED, tenant).await.unwrap();
    repos
        .agents
        .assign_agent(&kept, admin, admin)
        .await
        .unwrap();
    repos
        .agents
        .assign_agent(&retired, admin, admin)
        .await
        .unwrap();

    write_agent(checkout.path(), KEPT, Some(RETIRED));
    remove_agent(checkout.path(), RETIRED);
    assert!(
        seed(&repos, checkout.path(), false).await,
        "the unique pair does not fail the seed"
    );

    assert_eq!(agent_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        installs_for(&repos, &kept, admin).await,
        1,
        "the duplicate install cascades away, leaving exactly one"
    );
}

/// `training_plans` names its agent by slug and carries no foreign key, so
/// nothing cascades and nothing errors — an un-handed plan simply stops being
/// found, and the athlete's season is gone from every read.
#[tokio::test]
async fn a_merged_agent_hands_its_slug_keyed_plan_to_its_successor() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    plan_owned_by(&repos, admin, tenant, RETIRED).await;
    assert_eq!(
        plan_strategy(&repos, admin, tenant, RETIRED)
            .await
            .as_deref(),
        Some("Build, then taper.")
    );

    write_agent(checkout.path(), KEPT, Some(RETIRED));
    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(agent_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        plan_strategy(&repos, admin, tenant, KEPT).await.as_deref(),
        Some("Build, then taper."),
        "the athlete's season follows the successor"
    );
}

/// The store screen prints `store_listings.install_count`, a counter the
/// install path keeps rather than a figure read off the rows — so a
/// hand-over that moved the rows and left the counter alone would advertise
/// the successor as less installed than it is.
#[tokio::test]
async fn a_merged_agent_counts_the_installs_it_absorbed() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_agent(checkout.path(), KEPT, None);
    write_agent(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let kept = agent_id(&repos, KEPT, tenant).await.unwrap();
    let retired = agent_id(&repos, RETIRED, tenant).await.unwrap();
    repos
        .agents
        .assign_agent(&retired, admin, admin)
        .await
        .unwrap();
    assert_eq!(listing_install_count(&repos, &kept).await, 0);

    write_agent(checkout.path(), KEPT, Some(RETIRED));
    remove_agent(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(
        listing_install_count(&repos, &kept).await,
        1,
        "the counter follows the install it absorbed"
    );
}
