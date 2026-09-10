// ABOUTME: recommend_plan_flavour end to end — the four profile scenarios the plan sets as acceptance criteria, over the REAL catalogue
// ABOUTME: Pins that eligibility runs before ranking, that the profile scope gates the read, and that inputs echo with provenance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! These are content tests over the seeded `training_catalogue/` — the nine
//! flavours and forty-four selection rows as shipped — not fixtures. The
//! kernel's own tests cover the mechanics; what is asserted here is that the
//! data, read through the tool, produces the answers the plan promised.

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::scopes::{missing_scope, required_scopes};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("flavour_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

async fn recommend(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<Value> {
    let response = executor
        .execute_tool(request(
            "recommend_plan_flavour",
            params,
            user_id,
            tenant_id,
        ))
        .await?;
    assert!(
        response.success,
        "recommend_plan_flavour should have succeeded: {:?}",
        response.error
    );
    Ok(response.result.expect("a payload"))
}

fn top_id(payload: &Value) -> &str {
    payload["verdict"]["ranked"][0]["id"]
        .as_str()
        .expect("a ranked flavour")
}

fn excluded_reasons<'a>(payload: &'a Value, id: &str) -> Option<&'a Value> {
    payload["verdict"]["excluded"]
        .as_array()?
        .iter()
        .find(|e| e["id"] == id)
        .map(|e| &e["reasons"])
}

// ---------------------------------------------------------------------------
// The four scenarios the plan sets as acceptance criteria
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_novice_at_four_hours_never_gets_lactate_guided() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 4.0,
            "sessions_per_week": 4,
            "training_age": "novice",
            "interval_experience": "none",
            "sport_mix": "running",
        }),
    )
    .await?;

    let out = excluded_reasons(&payload, "norwegian-threshold-density")
        .expect("lactate-guided is excluded, not merely out-scored");
    assert!(
        out.as_array().is_some_and(|r| !r.is_empty()),
        "and the exclusion names its reasons: {out}"
    );
    assert_ne!(top_id(&payload), "norwegian-threshold-density");
    assert_eq!(
        top_id(&payload),
        "hvlit-foundation",
        "a novice gets foundation first: {}",
        payload["verdict"]["ranked"]
    );
    Ok(())
}

#[tokio::test]
async fn an_ironman_at_fourteen_hours_gets_a_pyramidal_base_with_a_durability_block() -> Result<()>
{
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 14.0,
            "sessions_per_week": 8,
            "training_age": "trained",
            "event_class": "ironman",
            "weeks_to_goal": 30,
            "interval_experience": "two_seasons",
            "measurements": ["power", "hr"],
            "sport_mix": "triathlon",
        }),
    )
    .await?;

    assert_eq!(
        top_id(&payload),
        "pyramidal-long-course",
        "the long-course pyramidal carries the durability block: {}",
        payload["verdict"]["ranked"]
    );
    // The season is laid on the ironman skeleton from the goal backward.
    assert_eq!(payload["season"]["status"], "laid", "{}", payload["season"]);
    let phases = payload["season"]["phases"].as_array().expect("phases");
    assert!(
        phases.iter().any(|p| p["kind"] == "taper"),
        "an ironman season ends in a taper: {phases:?}"
    );
    let total: u64 = phases
        .iter()
        .map(|p| p["weeks"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(total, 30, "every week of the runway belongs to a phase");
    Ok(())
}

#[tokio::test]
async fn a_five_k_runner_at_eight_hours_gets_polarized() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 8.0,
            "sessions_per_week": 6,
            "training_age": "trained",
            "event_class": "run_5k",
            "weeks_to_goal": 16,
            "interval_experience": "two_seasons",
            "measurements": ["hr", "pace"],
            "sport_mix": "running",
        }),
    )
    .await?;

    // "Leans polarized" is a claim about the hard work, and the catalogue
    // answers it two ways: polarized outright, or a pyramidal base that
    // becomes a polarized build. For a trained runner with two seasons of
    // intervals the table prefers the latter (Casado 2022), and the
    // run-5k-10k skeleton pins its build phase to polarized regardless. What
    // must never win here is a threshold-dense, lactate-guided, foundation
    // or plain-pyramidal flavour — that is the content of the KB's row.
    let top = top_id(&payload).to_owned();
    assert!(
        matches!(
            top.as_str(),
            "polarized-classic" | "race-specific" | "pyramidal-to-polarized"
        ),
        "a 5 km runner's hard work is polarized: {}",
        payload["verdict"]["ranked"]
    );
    assert_eq!(payload["season"]["status"], "laid", "{}", payload["season"]);
    let build = payload["season"]["phases"]
        .as_array()
        .expect("phases")
        .iter()
        .find(|p| p["kind"] == "build")
        .expect("a build phase");
    assert_eq!(
        build["flavour_override"], "polarized",
        "the 5 km skeleton pins its build to polarized whatever the season flavour: {build}"
    );
    Ok(())
}

#[tokio::test]
async fn an_athlete_with_no_meter_sees_lactate_guided_excluded_with_the_reason() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    // No measurements passed and none stored: read as effort.
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 12.0,
            "sessions_per_week": 8,
            "training_age": "elite",
            "interval_experience": "two_seasons",
            "sport_mix": "running",
        }),
    )
    .await?;

    let reasons = excluded_reasons(&payload, "norwegian-threshold-density")
        .expect("lactate-guided is excluded when there is nothing to measure with")
        .as_array()
        .expect("reasons")
        .clone();
    assert!(
        reasons
            .iter()
            .any(|r| r.as_str().is_some_and(|s| s.contains("lactate meter"))),
        "the catalogue's own words reach the athlete: {reasons:?}"
    );
    assert_eq!(
        payload["inputs"]["measurements"],
        json!(["rpe"]),
        "no device is read as effort, not as an unanswered dimension"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Provenance and the profile
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stored_thresholds_become_measurements_and_the_reply_says_so() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let saved = executor
        .execute_tool(request(
            "set_physiology",
            json!({ "ftp_watts": 265, "max_hr": 188, "fitness_level": "intermediate" }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(saved.success, "{:?}", saved.error);

    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({ "hours_per_week": 9.0, "sessions_per_week": 6, "sport_mix": "cycling" }),
    )
    .await?;

    let measured = payload["inputs"]["measurements"]
        .as_array()
        .expect("measurements");
    assert!(
        measured.contains(&json!("power")),
        "FTP means a power meter: {measured:?}"
    );
    assert!(
        measured.contains(&json!("hr")),
        "max HR means a strap: {measured:?}"
    );
    let sources = payload["inputs"]["sources"].as_array().expect("sources");
    let from = |input: &str| {
        sources
            .iter()
            .find(|s| s["input"] == input)
            .map_or("", |s| s["from"].as_str().unwrap_or(""))
    };
    assert_eq!(from("measurements"), "profile", "{sources:?}");
    assert_eq!(from("training_age"), "profile", "{sources:?}");
    assert_eq!(
        payload["inputs"]["training_age"], "trained",
        "intermediate is trained"
    );
    assert_eq!(from("sport_mix"), "argument", "{sources:?}");
    assert_eq!(from("recovery_speed"), "default", "{sources:?}");
    Ok(())
}

#[tokio::test]
async fn no_goal_race_means_no_season_and_says_so() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({ "hours_per_week": 6.0, "sessions_per_week": 5 }),
    )
    .await?;
    assert_eq!(
        payload["season"]["status"], "no_goal",
        "{}",
        payload["season"]
    );
    assert!(
        payload["verdict"]["missing_inputs"]
            .as_array()
            .is_some_and(|m| m.iter().any(|d| d == "event_class")),
        "the verdict names what it could not answer: {}",
        payload["verdict"]["missing_inputs"]
    );
    Ok(())
}

#[tokio::test]
async fn too_little_runway_is_refused_rather_than_compressed() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 8.0,
            "sessions_per_week": 5,
            "training_age": "trained",
            "event_class": "marathon",
            "weeks_to_goal": 4,
        }),
    )
    .await?;
    assert_eq!(
        payload["season"]["status"], "not_enough_runway",
        "{}",
        payload["season"]
    );
    assert!(payload["season"]["needs_weeks"].as_u64().unwrap_or(0) > 4);
    Ok(())
}

#[tokio::test]
async fn an_impossible_payload_is_refused_as_input() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let rejected = match executor
        .execute_tool(request(
            "recommend_plan_flavour",
            json!({ "hours_per_week": 200.0, "sessions_per_week": 5 }),
            user_id,
            &tenant_id,
        ))
        .await
    {
        Err(e) => e.to_string(),
        Ok(response) => {
            assert!(
                !response.success,
                "200 hours a week is a typo, not a profile: {:?}",
                response.result
            );
            format!("{:?}", response.error)
        }
    };
    assert!(
        rejected.contains("hours_per_week"),
        "the refusal names the field: {rejected}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_vocabulary_word_lists_the_real_ones() -> Result<()> {
    // The schema names the words in prose, not as `enum_values`; the typed
    // payload is what refuses a word outside the kernel's vocabulary.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let response = executor
        .execute_tool(request(
            "recommend_plan_flavour",
            json!({ "hours_per_week": 6.0, "sessions_per_week": 4, "training_age": "veteran" }),
            user_id,
            &tenant_id,
        ))
        .await;
    let text = match response {
        Err(e) => e.to_string(),
        Ok(r) => {
            assert!(!r.success, "'veteran' is not a band: {:?}", r.result);
            format!("{:?} {:?}", r.error, r.result)
        }
    };
    assert!(
        text.contains("veteran"),
        "the refusal names the word: {text}"
    );
    for band in ["novice", "recreational", "trained", "elite"] {
        assert!(text.contains(band), "the refusal lists {band}: {text}");
    }
    Ok(())
}

#[tokio::test]
async fn every_flavour_in_the_verdict_carries_its_plain_words_label() -> Result<()> {
    // D9: the athlete hears "mostly easy with two hard days", never
    // "polarized". The id stays beside the label for the coach.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({ "hours_per_week": 8.0, "sessions_per_week": 5, "training_age": "trained",
                "event_class": "run_5k", "weeks_to_goal": 10, "measurements": ["hr", "pace"],
                "interval_experience": "two_seasons", "sport_mix": "running" }),
    )
    .await?;
    for list in ["ranked", "excluded"] {
        let entries = payload["verdict"][list].as_array().expect(list);
        assert!(!entries.is_empty(), "{list} is empty: {payload}");
        for entry in entries {
            let id = entry["id"].as_str().expect("id");
            let label = entry["label"].as_str().expect("label");
            assert_ne!(label, id, "{id} has no plain-words label in the catalogue");
            assert!(
                !label.contains("polarized") && !label.contains("pyramidal"),
                "{id}'s label is the technical word again: {label}"
            );
        }
    }
    let polarized = payload["verdict"]["ranked"]
        .as_array()
        .expect("ranked")
        .iter()
        .chain(payload["verdict"]["excluded"].as_array().expect("excluded"))
        .find(|e| e["id"] == "polarized-classic")
        .expect("polarized-classic is in the verdict one way or the other");
    // The test user's locale is the default, French.
    assert_eq!(
        polarized["label"],
        "surtout du facile avec deux journées dures"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The `athlete` argument: a human coach reading a consenting athlete
// ---------------------------------------------------------------------------

/// A user with a display name of their own, owning one tenant — the shape
/// `resolve_plan_scope` matches a roster query against and files under.
async fn seed_named_user(
    executor: &UniversalToolExecutor,
    display_name: &str,
) -> Result<(Uuid, TenantId)> {
    let repos = executor.resources.repos();
    let mut user = User::new(
        format!("flavour_scope_{}@example.com", Uuid::new_v4()),
        bcrypt::hash("Pass123!", 4)?,
        Some(display_name.to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await?;
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{display_name}'s tenant"),
            slug: format!("flavour-scope-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await?;
    repos.users.update_tenant_id(user_id, tenant_id).await?;
    Ok((user_id, tenant_id))
}

/// A group the coach is attached to as its human coach, with the athlete a
/// consenting member and peer sharing on — every gate the resolver walks, open.
async fn attach_as_coach(
    executor: &UniversalToolExecutor,
    coach: Uuid,
    coach_tenant: TenantId,
    athlete: Uuid,
    athlete_tenant: TenantId,
) -> Result<()> {
    let repos = executor.resources.repos();
    let persona = repos
        .coaches
        .create_system_coach(
            coach,
            coach_tenant,
            &CreateSystemCoachRequest {
                title: "Flavour Scope Coach".to_owned(),
                description: None,
                system_prompt: "Test prompt".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: CoachVisibility::Global,
            },
        )
        .await?
        .id;
    let group_id = Uuid::new_v4();
    let now = Utc::now();
    repos
        .groups
        .create_group(
            coach_tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: coach_tenant.to_string(),
                name: "Flavour Squad".to_owned(),
                description: None,
                coach_id: persona.to_string(),
                owner_id: coach,
                coach_user_id: Some(coach),
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::default(),
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
    repos
        .groups
        .set_group_coach_user(&group_id.to_string(), Some(coach), coach_tenant)
        .await?;
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id: athlete,
            tenant_id: athlete_tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: true,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_coach_reads_the_consenting_athletes_thresholds_not_their_own() -> Result<()> {
    let executor = create_executor().await?;
    let (coach, coach_tenant) = seed_named_user(&executor, "Coach Karine").await?;
    let (athlete, athlete_tenant) = seed_named_user(&executor, "Phil Tremblay").await?;
    attach_as_coach(&executor, coach, coach_tenant, athlete, athlete_tenant).await?;

    // Only the athlete has thresholds on file. Read through the coach, the
    // devices must be the athlete's — the coach's own empty profile would
    // resolve to effort alone.
    let saved = executor
        .execute_tool(request(
            "set_physiology",
            json!({ "ftp_watts": 240, "fitness_level": "advanced" }),
            athlete,
            &athlete_tenant.to_string(),
        ))
        .await?;
    assert!(saved.success, "{:?}", saved.error);

    let payload = recommend(
        &executor,
        coach,
        &coach_tenant.to_string(),
        json!({ "hours_per_week": 8.0, "sessions_per_week": 5, "athlete": "phil" }),
    )
    .await?;

    assert_eq!(
        payload["athlete"], "Phil Tremblay",
        "the reply names whose profile was read: {payload}"
    );
    let measured = payload["inputs"]["measurements"]
        .as_array()
        .expect("measurements");
    assert!(
        measured.contains(&json!("power")),
        "the athlete's FTP, not the coach's empty profile: {measured:?}"
    );
    assert_eq!(
        payload["inputs"]["training_age"], "trained",
        "advanced is trained"
    );
    Ok(())
}

#[tokio::test]
async fn an_athlete_the_caller_does_not_coach_is_refused_by_name() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let response = executor
        .execute_tool(request(
            "recommend_plan_flavour",
            json!({ "hours_per_week": 8.0, "sessions_per_week": 5, "athlete": "Nobody Here" }),
            user_id,
            &tenant_id,
        ))
        .await?;
    let text = format!("{:?} {:?}", response.error, response.result);
    assert!(
        text.contains("No athlete matching 'Nobody Here'"),
        "the refusal names the query and the rule: {text}"
    );
    assert!(
        !text.contains("\"verdict\""),
        "a refused read carries no verdict: {text}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The profile read is scope-gated — declared from the first commit
// ---------------------------------------------------------------------------

#[test]
fn the_tool_declares_the_profile_read_it_performs() {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let (_, _, caps, _) = registry
        .all_tool_metadata()
        .into_iter()
        .find(|(name, _, _, _)| name == "recommend_plan_flavour")
        .expect("the tool is registered");

    assert_eq!(
        required_scopes(caps),
        vec![OAuthScope::ProfileRead],
        "reading training age, thresholds and sport requires profile:read"
    );
    assert_eq!(
        missing_scope(&[OAuthScope::FitnessRead], caps),
        Some(OAuthScope::ProfileRead),
        "a fitness-read grant must be refused and told what it needed"
    );
    assert_eq!(missing_scope(&OAuthScope::self_grant(), caps), None);
}

// ---------------------------------------------------------------------------
// Two A races: the season the kernel lays for a calendar, not for one date
// ---------------------------------------------------------------------------

/// Save an outline whose calendar carries a second A race, then read the season
/// the tool lays for it.
async fn save_two_a_race_outline(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    first_a: &str,
    goal_a: &str,
) -> Result<()> {
    let response = executor
        .execute_tool(request(
            "save_training_plan",
            json!({
                "outline": {
                    "goal_race": { "name": "Autumn marathon", "date": goal_a, "discipline": "marathon", "priority": "A" },
                    "races": [
                        { "name": "Summer half", "date": first_a, "discipline": "half_marathon", "priority": "A" }
                    ],
                    "strategy": "two peaks, the half first",
                    "phases": []
                },
                "weeks": []
            }),
            user_id,
            tenant_id,
        ))
        .await?;
    assert!(
        response.success,
        "the two-race outline should save: {:?}",
        response.error
    );
    Ok(())
}

#[tokio::test]
async fn a_second_a_race_becomes_a_second_peak_with_a_transition_between() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let today = Utc::now().date_naive();
    let first_a = (today + chrono::Duration::weeks(20))
        .format("%Y-%m-%d")
        .to_string();
    let goal_a = (today + chrono::Duration::weeks(36))
        .format("%Y-%m-%d")
        .to_string();
    save_two_a_race_outline(&executor, user_id, &tenant_id, &first_a, &goal_a).await?;

    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 8.0,
            "sessions_per_week": 5,
            "training_age": "trained",
            "event_class": "marathon",
        }),
    )
    .await?;

    assert_eq!(payload["season"]["status"], "laid", "{}", payload["season"]);
    let phases = payload["season"]["phases"]
        .as_array()
        .expect("a laid season lists its phases");
    let kinds: Vec<&str> = phases.iter().filter_map(|p| p["kind"].as_str()).collect();
    assert!(
        kinds.contains(&"transition"),
        "the race between today and the goal is absorbed before the next block: {kinds:?}"
    );

    // The transition sits between the two blocks, never first and never last:
    // it exists to absorb a race that has already been run.
    let at = kinds
        .iter()
        .position(|k| *k == "transition")
        .expect("the transition was just asserted");
    assert!(
        at > 0,
        "nothing is absorbed before the first race: {kinds:?}"
    );
    assert!(
        at + 1 < kinds.len(),
        "a season does not end on a transition: {kinds:?}"
    );

    // Both peaks are laid on: a phase ends on each race.
    let peaks: Vec<&str> = phases.iter().filter_map(|p| p["peak"].as_str()).collect();
    assert!(
        peaks.contains(&first_a.as_str()),
        "the first A race is a peak: {peaks:?}"
    );
    assert!(
        peaks.contains(&goal_a.as_str()),
        "the goal race is a peak: {peaks:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_b_race_is_not_a_peak() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let today = Utc::now().date_naive();
    let b_race = (today + chrono::Duration::weeks(20))
        .format("%Y-%m-%d")
        .to_string();
    let goal_a = (today + chrono::Duration::weeks(30))
        .format("%Y-%m-%d")
        .to_string();

    let response = executor
        .execute_tool(request(
            "save_training_plan",
            json!({
                "outline": {
                    "goal_race": { "name": "Autumn marathon", "date": goal_a, "discipline": "marathon", "priority": "A" },
                    "races": [
                        { "name": "Club 10k", "date": b_race, "discipline": "run_10k", "priority": "B" }
                    ],
                    "strategy": "one peak, a tune-up on the way",
                    "phases": []
                },
                "weeks": []
            }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(response.success, "{:?}", response.error);

    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 8.0,
            "sessions_per_week": 5,
            "training_age": "trained",
            "event_class": "marathon",
        }),
    )
    .await?;

    let phases = payload["season"]["phases"]
        .as_array()
        .expect("a laid season lists its phases");
    let kinds: Vec<&str> = phases.iter().filter_map(|p| p["kind"].as_str()).collect();
    assert!(
        !kinds.contains(&"transition"),
        "a B race is ridden through, not peaked for: {kinds:?}"
    );
    let peaks: Vec<&str> = phases.iter().filter_map(|p| p["peak"].as_str()).collect();
    assert!(
        !peaks.contains(&b_race.as_str()),
        "the B race is not a peak: {peaks:?}"
    );
    Ok(())
}

#[tokio::test]
async fn the_laid_season_never_goes_backwards() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let today = Utc::now().date_naive();
    let first_a = (today + chrono::Duration::weeks(20))
        .format("%Y-%m-%d")
        .to_string();
    let goal_a = (today + chrono::Duration::weeks(36))
        .format("%Y-%m-%d")
        .to_string();
    save_two_a_race_outline(&executor, user_id, &tenant_id, &first_a, &goal_a).await?;

    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 8.0,
            "sessions_per_week": 5,
            "training_age": "trained",
            "event_class": "marathon",
        }),
    )
    .await?;

    // The report calls itself "the phases, earliest first". A second block laid
    // backward through the first prescribed a taper and a base for the same
    // week, and read as a plan because nothing checked the dates.
    let starts: Vec<&str> = payload["season"]["phases"]
        .as_array()
        .expect("phases")
        .iter()
        .filter_map(|p| p["start"].as_str())
        .collect();
    let mut ordered = starts.clone();
    ordered.sort_unstable();
    assert_eq!(starts, ordered, "phases must be in calendar order");
    Ok(())
}

#[tokio::test]
async fn an_a_race_too_soon_for_a_block_is_named_not_silently_dropped() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let today = Utc::now().date_naive();
    // Six weeks after the first: under the marathon skeleton's twelve-week
    // floor, so it cannot carry a block of its own.
    let crowded = (today + chrono::Duration::weeks(26))
        .format("%Y-%m-%d")
        .to_string();
    let goal_a = (today + chrono::Duration::weeks(20))
        .format("%Y-%m-%d")
        .to_string();
    save_two_a_race_outline(&executor, user_id, &tenant_id, &crowded, &goal_a).await?;

    let payload = recommend(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "hours_per_week": 8.0,
            "sessions_per_week": 5,
            "training_age": "trained",
            "event_class": "marathon",
        }),
    )
    .await?;

    assert_eq!(payload["season"]["status"], "laid", "{}", payload["season"]);
    let unlaid: Vec<&str> = payload["season"]["unlaid_peaks"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        unlaid,
        vec![crowded.as_str()],
        "the coach is told which race the season is not built toward: {}",
        payload["season"]
    );

    let starts: Vec<&str> = payload["season"]["phases"]
        .as_array()
        .expect("phases")
        .iter()
        .filter_map(|p| p["start"].as_str())
        .collect();
    let mut ordered = starts.clone();
    ordered.sort_unstable();
    assert_eq!(starts, ordered, "and the season it did lay is in order");
    Ok(())
}
