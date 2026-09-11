// ABOUTME: Agent package artefacts — parsed from the agent directory, stored per agent, lent to a plan only when published
// ABOUTME: Package over catalogue over compiled-in at every read, provenance stamped on the saved day, flags at review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The second authorship tier end to end: `read_package_artefacts` on a
//! directory laid out like a contremaitre agent, the rows the seeder writes,
//! the `PublishStatus` gate in `load_agent_package`, the shadowing rules of
//! `PackagedCatalogue`, the review report, and the `template_source` a saved
//! day carries.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_agent_parser::read_package_artefacts;
use pierre_contremaitre::{EvidenceRegistry, TrainingCatalogueRegistry};
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::periodization::WorkoutFilter;
use pierre_core::models::{ArtefactKind, PackageArtefact, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::RepositoryRegistry;
use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_memory::training_plans::TemplateSource;
use pierre_services::agent_package::{
    load_agent_package, review_package, CatalogueTier, PackagedCatalogue,
};
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::plan_flavour::RecommendPlanFlavourTool;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;
mod helpers;

use helpers::agent_fixtures::publish_catalogue_agent_tagged;
use helpers::axum_test::AxumTestRequest;
use pierre_database::repositories::training_plans::PlanOwner;
use pierre_mcp_server::routes::endurance::endurance_routes;

const CATALOGUE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../training_catalogue");

/// The catalogue's polarized flavour, re-labelled as a house flavour with
/// its own id — a flavour no selection row names.
fn house_flavour_yaml() -> String {
    let text = fs::read_to_string(Path::new(CATALOGUE_DIR).join("flavours/polarized-classic.yaml"))
        .expect("catalogue flavour readable");
    text.replace("id: polarized-classic", "id: house-polarized")
}

/// The catalogue's polarized flavour under its own id, with a tighter hard
/// session cap — an override of a catalogue flavour.
fn override_flavour_yaml() -> String {
    let text = fs::read_to_string(Path::new(CATALOGUE_DIR).join("flavours/polarized-classic.yaml"))
        .expect("catalogue flavour readable");
    text.replace("  min: 1\n  max: 2\n", "  min: 1\n  max: 1\n")
}

/// The catalogue's threshold session under a package slug.
fn package_workout_toml(slug: &str) -> String {
    let text = fs::read_to_string(Path::new(CATALOGUE_DIR).join("workouts/threshold_4x8.toml"))
        .expect("catalogue workout readable");
    text.replace("slug = \"threshold_4x8\"", &format!("slug = \"{slug}\""))
        .replace(
            "id = \"00000000-0000-0000-0000-000000000002\"",
            &format!("id = \"{}\"", Uuid::new_v4()),
        )
}

/// A grey-practice session that cites nothing — the kernel admits it with a
/// caveat, and the review flags it.
fn uncited_workout_toml(slug: &str) -> String {
    package_workout_toml(slug)
        .replace("evidence_tier = \"review\"", "evidence_tier = \"grey\"\ncaveat = \"House convention; no proposition in the corpus speaks to it.\"")
        .replace(
            "evidence_refs = [\n  \"evidence/sports_science/training_prescription/tonnessen-2024-norwegian-coach-session-models.md\",\n]",
            "evidence_refs = []",
        )
}

/// An agent directory laid out like contremaitre's: `en.md` beside the package.
fn coach_dir(name: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!("coach-package-{name}-{}", Uuid::new_v4()));
    fs::create_dir_all(dir.join("workouts")).expect("temp coach dir");
    fs::write(dir.join("en.md"), "---\nname: x\n---\n").expect("write en.md");
    dir
}

#[test]
fn a_coach_directory_yields_its_artefacts_in_order() {
    let dir = coach_dir("full");
    fs::write(dir.join("flavour.yaml"), house_flavour_yaml()).unwrap();
    fs::write(
        dir.join("skeleton.yaml"),
        fs::read_to_string(Path::new(CATALOGUE_DIR).join("skeletons/marathon-linear.yaml"))
            .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("workouts/house_4x8.toml"),
        package_workout_toml("house_4x8"),
    )
    .unwrap();
    fs::write(
        dir.join("workouts/house_easy.toml"),
        uncited_workout_toml("house_easy"),
    )
    .unwrap();

    let artefacts = read_package_artefacts(&dir).expect("package parses");
    let listed: Vec<(ArtefactKind, &str)> = artefacts
        .iter()
        .map(|a| (a.kind, a.slug.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (ArtefactKind::Flavour, "house-polarized"),
            (ArtefactKind::Skeleton, "marathon-linear"),
            (ArtefactKind::Workout, "house_4x8"),
            (ArtefactKind::Workout, "house_easy"),
        ]
    );
    assert_eq!(artefacts[0].sha256.len(), 64);
    assert!(artefacts[2].content.contains("slug = \"house_4x8\""));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_directory_with_only_prompts_yields_nothing() {
    let dir = coach_dir("bare");
    fs::remove_dir_all(dir.join("workouts")).unwrap();
    assert!(read_package_artefacts(&dir)
        .expect("no package is fine")
        .is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_workout_file_named_unlike_its_slug_is_refused() {
    let dir = coach_dir("misnamed");
    fs::write(
        dir.join("workouts/other.toml"),
        package_workout_toml("house_4x8"),
    )
    .unwrap();
    let err = read_package_artefacts(&dir).expect_err("stem must match slug");
    let msg = err.to_string();
    assert!(msg.contains("house_4x8") && msg.contains("other"), "{msg}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_the_kernel_refuses_names_itself() {
    let dir = coach_dir("broken");
    fs::write(dir.join("flavour.yaml"), "id: broken\nfamily: nonsense\n").unwrap();
    let err = read_package_artefacts(&dir).expect_err("kernel refuses");
    let msg = err.to_string();
    assert!(
        msg.contains("flavour.yaml") && msg.contains("flavour"),
        "{msg}"
    );
    fs::remove_dir_all(&dir).ok();
}

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    let email = format!("coach_package_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id))
}

/// A published agent carrying `flavour_yaml` as its one flavour, plus one
/// cited and one uncited workout.
async fn published_coach_with_package(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    flavour_yaml: &str,
) -> Uuid {
    let agent_id = publish_catalogue_agent_tagged(
        repos,
        user_id,
        tenant,
        "House Polarized",
        "You coach the house way.",
        AgentCategory::Training,
        vec!["polarized".to_owned()],
    )
    .await;
    let artefacts = vec![
        PackageArtefact::parse(ArtefactKind::Flavour, flavour_yaml).unwrap(),
        PackageArtefact::parse(ArtefactKind::Workout, &package_workout_toml("house_4x8")).unwrap(),
        PackageArtefact::parse(ArtefactKind::Workout, &uncited_workout_toml("house_easy")).unwrap(),
    ];
    let stored = repos
        .agent_artefacts
        .replace_agent_artefacts(&tenant.to_string(), &agent_id.to_string(), &artefacts)
        .await
        .unwrap();
    assert_eq!(stored, 3);
    agent_id
}

#[tokio::test]
async fn the_repository_replaces_the_set_and_lists_it_in_order() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &house_flavour_yaml()).await;

    let rows = repos
        .agent_artefacts
        .list_agent_artefacts(&tenant.to_string(), &agent_id.to_string())
        .await?;
    let listed: Vec<(ArtefactKind, &str)> =
        rows.iter().map(|r| (r.kind, r.slug.as_str())).collect();
    assert_eq!(
        listed,
        vec![
            (ArtefactKind::Flavour, "house-polarized"),
            (ArtefactKind::Workout, "house_4x8"),
            (ArtefactKind::Workout, "house_easy"),
        ]
    );
    assert!(rows.iter().all(|r| r.tenant_id == tenant.to_string()));

    // Replacing with a smaller set drops what is gone; an empty set clears.
    let only_flavour = vec![PackageArtefact::parse(
        ArtefactKind::Flavour,
        &house_flavour_yaml(),
    )?];
    repos
        .agent_artefacts
        .replace_agent_artefacts(&tenant.to_string(), &agent_id.to_string(), &only_flavour)
        .await?;
    let rows = repos
        .agent_artefacts
        .list_agent_artefacts(&tenant.to_string(), &agent_id.to_string())
        .await?;
    assert_eq!(rows.len(), 1);
    repos
        .agent_artefacts
        .replace_agent_artefacts(&tenant.to_string(), &agent_id.to_string(), &[])
        .await?;
    assert!(repos
        .agent_artefacts
        .list_agent_artefacts(&tenant.to_string(), &agent_id.to_string())
        .await?
        .is_empty());
    Ok(())
}

#[tokio::test]
async fn a_package_is_lent_only_while_its_listing_is_published() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &house_flavour_yaml()).await;
    let id = agent_id.to_string();

    let package = load_agent_package(repos, tenant, user_id, Some(&id))
        .await?
        .expect("published package resolves");
    assert_eq!(package.house_flavour(), Some("house-polarized"));
    assert_eq!(package.workouts.len(), 2);

    // Unpublished: the same rows lend nothing.
    repos.store_listings.unpublish_agent(&id, tenant).await?;
    assert!(load_agent_package(repos, tenant, user_id, Some(&id))
        .await?
        .is_none());

    // No agent at all: nothing, not an error.
    assert!(load_agent_package(repos, tenant, user_id, None)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn an_installed_copy_reads_the_origin_package() -> Result<()> {
    let executor = create_executor().await?;
    let (author, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let origin = published_coach_with_package(repos, author, tenant, &house_flavour_yaml()).await;
    let copy =
        helpers::agent_fixtures::install_catalogue_agent(repos, origin, author, tenant).await;
    assert_eq!(copy.forked_from, Some(origin));

    let package = load_agent_package(repos, tenant, author, Some(&copy.id.to_string()))
        .await?
        .expect("the copy resolves its origin's package");
    assert_eq!(package.agent_id, origin.to_string());
    Ok(())
}

#[tokio::test]
async fn package_over_catalogue_over_compiled_in() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &override_flavour_yaml()).await;
    let package = load_agent_package(repos, tenant, user_id, Some(&agent_id.to_string()))
        .await?
        .expect("package");

    let registry = TrainingCatalogueRegistry::new();
    let view = PackagedCatalogue::new(&registry, Some(package));

    // The override shadows the catalogue flavour of the same id.
    let (flavour, tier) = view.flavour("polarized-classic").expect("flavour resolves");
    assert_eq!(tier, CatalogueTier::Package);
    assert_eq!(flavour.hard_sessions_per_week.max, 1);
    assert_eq!(
        registry
            .flavour("polarized-classic")
            .unwrap()
            .hard_sessions_per_week
            .max,
        2,
        "the registry itself is untouched"
    );
    let ids: Vec<String> = view.flavours().into_iter().map(|f| f.id).collect();
    assert_eq!(
        ids.iter().filter(|id| *id == "polarized-classic").count(),
        1,
        "shadowed, not duplicated: {ids:?}"
    );
    assert_eq!(ids.len(), registry.flavours().len());

    // A catalogue flavour the package does not carry still answers.
    let (_, tier) = view.flavour("pyramidal-base").expect("catalogue flavour");
    assert_eq!(tier, CatalogueTier::Catalogue);

    // Package workouts resolve as package; catalogue ones as catalogue.
    let (w, tier) = view.workout("house_4x8").expect("package workout");
    assert_eq!(tier, CatalogueTier::Package);
    assert_eq!(w.slug, "house_4x8");
    let (_, tier) = view.workout("threshold_4x8").expect("catalogue workout");
    assert_eq!(tier, CatalogueTier::Catalogue);
    assert!(view.workout("nowhere").is_none());

    // The filtered list puts package templates first, then the catalogue's.
    let threshold = view.workouts_matching(&WorkoutFilter {
        purpose: Some(w.purpose),
        phase: None,
        sport: None,
    });
    assert_eq!(threshold[0].slug, "house_4x8");
    assert!(threshold.iter().any(|t| t.slug == "threshold_4x8"));

    // A view with no package is the registry.
    let bare = PackagedCatalogue::catalogue_only(&registry);
    assert!(bare.workout("house_4x8").is_none());
    assert_eq!(bare.house_flavour(), None);
    Ok(())
}

#[tokio::test]
async fn the_review_flags_what_nothing_answers() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &house_flavour_yaml()).await;
    let rows = repos
        .agent_artefacts
        .list_agent_artefacts(&tenant.to_string(), &agent_id.to_string())
        .await?;
    let registry = TrainingCatalogueRegistry::new();

    // An empty evidence registry checks nothing and says so.
    let unchecked = review_package(&rows, &registry, &EvidenceRegistry::new());
    assert!(!unchecked.evidence_checked);
    assert_eq!(unchecked.artefacts.len(), 3);
    assert!(unchecked.artefacts.iter().all(|a| a.unresolved.is_empty()));
    let uncited = unchecked
        .artefacts
        .iter()
        .find(|a| a.slug == "house_easy")
        .expect("uncited workout listed");
    assert!(uncited.cites_no_evidence);
    assert!(
        !unchecked
            .artefacts
            .iter()
            .find(|a| a.slug == "house_4x8")
            .unwrap()
            .cites_no_evidence
    );

    // A registry that holds one proposition flags every path it lacks.
    let evidence = EvidenceRegistry::new();
    evidence.update(
        "training_prescription",
        "some-other-proposition",
        EvidenceCorpus::default(),
        "0".repeat(64),
    );
    let checked = review_package(&rows, &registry, &evidence);
    assert!(checked.evidence_checked);
    let cited = checked
        .artefacts
        .iter()
        .find(|a| a.slug == "house_4x8")
        .expect("cited workout listed");
    assert_eq!(cited.unresolved.len(), 1, "{:?}", cited.unresolved);
    assert_eq!(cited.unresolved[0].key, "evidence_refs[0]");
    assert!(cited.unresolved[0]
        .reference
        .ends_with("tonnessen-2024-norwegian-coach-session-models.md"));
    let flavour = checked
        .artefacts
        .iter()
        .find(|a| a.kind == ArtefactKind::Flavour)
        .expect("flavour listed");
    assert!(
        !flavour.unresolved.is_empty(),
        "the flavour cites propositions the registry lacks"
    );
    Ok(())
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

fn day(date: &str, slug: Option<&str>) -> Value {
    let mut d = json!({
        "date": date,
        "sport": "run",
        "workout": "4 x 8 min at threshold",
        "duration_min": 75,
        "intensity": "threshold",
    });
    if let Some(slug) = slug {
        d["template_slug"] = json!(slug);
        // Whatever the payload claims about provenance is overwritten.
        d["template_source"] = json!("athlete");
    }
    d
}

fn plan_payload(agent_id: &str, days: &[Value]) -> Value {
    json!({
        "agent_id": agent_id,
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": "2026-11-14", "discipline": "run_5k", "priority": "A" },
            "strategy": "house polarized build",
            "flavour": { "id": "house-polarized", "selected_by": "coach", "override_reason": "house style" },
            "phases": [
                {"kind": "build", "start": "2026-09-14", "weeks": 6, "intent": "two hard days", "target_hours": 8.0},
                {"kind": "taper", "start": "2026-11-02", "weeks": 2, "intent": "sharpen"}
            ]
        },
        "weeks": [{
            "week_start": "2026-09-14",
            "focus": "first build week",
            "phase_index": 0,
            "days": days
        }]
    })
}

#[tokio::test]
async fn a_saved_day_records_the_tier_its_template_came_from() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &house_flavour_yaml()).await;
    let id = agent_id.to_string();

    let days = vec![
        day("2026-09-15", Some("house_4x8")),
        day("2026-09-17", Some("threshold_4x8")),
        day("2026-09-19", None),
    ];
    let save = executor
        .execute_tool(request(
            "save_training_plan",
            plan_payload(&id, &days),
            user_id,
            tenant,
        ))
        .await?;
    assert!(save.success, "save failed: {:?}", save.error);

    let plan = repos
        .training_plans
        .get_active_plan(
            &tenant.to_string(),
            &user_id.to_string(),
            PlanOwner::agent(&id),
        )
        .await?
        .expect("plan saved under the coach");
    assert_eq!(
        plan.flavour.as_ref().map(|f| f.id.as_str()),
        Some("house-polarized"),
        "a house flavour the catalogue lacks is accepted through the package"
    );
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant.to_string(), &user_id.to_string(), &plan.id, false)
        .await?;
    let sources: Vec<(Option<&str>, Option<TemplateSource>)> = weeks[0]
        .days
        .iter()
        .map(|d| (d.template_slug.as_deref(), d.template_source))
        .collect();
    assert_eq!(
        sources,
        vec![
            (Some("house_4x8"), Some(TemplateSource::Package)),
            (Some("threshold_4x8"), Some(TemplateSource::Catalogue)),
            (None, None),
        ]
    );

    // Unpublish the agent: the house flavour is no longer a flavour the
    // save accepts, and the package template is no longer one either.
    repos.store_listings.unpublish_agent(&id, tenant).await?;
    let again = executor
        .execute_tool(request(
            "save_training_plan",
            plan_payload(&id, &[day("2026-09-15", Some("house_4x8"))]),
            user_id,
            tenant,
        ))
        .await;
    let msg = match again {
        Err(e) => e.to_string(),
        Ok(response) => {
            assert!(!response.success, "an unpublished package lends nothing");
            response.error.unwrap_or_default()
        }
    };
    assert!(msg.contains("house-polarized"), "{msg}");
    Ok(())
}

#[tokio::test]
async fn the_rule_pins_the_house_flavour_with_package_provenance() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = executor.resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &house_flavour_yaml()).await;

    // The tool reads the agent from the conversation, never from an
    // argument, so the call runs inside a chat with that agent.
    let conversation = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "dm",
            "gemini-2.0-flash",
            Some(&agent_id.to_string()),
            None,
        )
        .await?;
    assert_eq!(
        conversation.agent_id.as_deref(),
        Some(agent_id.to_string().as_str()),
        "the conversation names the coach"
    );
    let found = repos
        .chat
        .get_conversation(&conversation.id, &user_id.to_string(), tenant)
        .await?;
    assert!(found.is_some(), "the athlete is a participant");
    // The executor runs a tool on its own task, where a task-local scoped
    // here is out of reach; the tool is called directly, the way the chat
    // dispatch does inside the scope.
    let runtime: Arc<dyn ToolRuntime> = executor.resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = CONVERSATION_ID
        .scope(Some(conversation.id), async {
            RecommendPlanFlavourTool
                .execute(
                    &runtime,
                    &ctx,
                    json!({
                        "hours_per_week": 8.0,
                        "sessions_per_week": 5,
                        "training_age": "trained",
                        "interval_experience": "two_seasons",
                        "measurements": ["hr"]
                    }),
                )
                .await
        })
        .await;
    let result = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        result.get("error").is_none(),
        "tool failed: {}",
        result["error"]
    );
    let inputs = &result["inputs"];
    assert_eq!(inputs["coach_preference"], json!("house-polarized"));
    let sources = inputs["sources"].as_array().expect("sources");
    assert!(
        sources
            .iter()
            .any(|s| s["input"] == "coach_preference" && s["from"] == "package"),
        "{sources:?}"
    );
    assert_eq!(
        result["verdict"]["coach_pinned"],
        json!("house-polarized"),
        "{}",
        result["verdict"]
    );
    assert_eq!(
        result["verdict"]["ranked"][0]["id"],
        json!("house-polarized")
    );
    Ok(())
}

#[tokio::test]
async fn the_rest_template_list_lays_the_selected_coachs_package_over_the_catalogue() -> Result<()>
{
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor =
        Arc::new(UniversalToolExecutor::new(runtime).with_scopes(OAuthScope::self_grant()));
    let (user_id, tenant) = create_test_user(&executor).await?;
    let repos = resources.repos();
    let agent_id =
        published_coach_with_package(repos, user_id, tenant, &house_flavour_yaml()).await;
    repos
        .tenants
        .set_selected_agent(tenant, user_id, Some(&agent_id.to_string()))
        .await?;

    let user = repos
        .users
        .get_global(user_id)
        .await?
        .expect("the athlete exists");
    let token = common::generate_test_token(&resources, &user).await;
    let router = endurance_routes().with_state(Arc::clone(&resources));
    let response = AxumTestRequest::get("/api/v1/endurance/workout-templates")
        .header("Authorization", &format!("Bearer {token}"))
        .send(router)
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let slugs: Vec<&str> = body
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|t| t["slug"].as_str())
        .collect();
    assert_eq!(
        slugs[0], "house_4x8",
        "package templates come first: {slugs:?}"
    );
    assert!(slugs.contains(&"house_easy"));
    assert!(
        slugs.contains(&"threshold_4x8"),
        "the catalogue is still listed"
    );
    Ok(())
}
