// ABOUTME: /fortnight end to end — the platform's decision reaches the athlete in their own language
// ABOUTME: Every refusal names what would change it, and none of them claims a readiness the command never read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The command decides in Rust and speaks through the string catalogue, so
//! these assert on what the athlete actually reads rather than on an
//! identifier. A refusal that renders an empty string, or renders the key
//! itself, passes an `is_ok()` check and fails a human.

use anyhow::Result;
use chrono::Utc;
use helpers::coach_fixtures::publish_catalogue_coach;
use pierre_chat_pipeline::stages::onboarding::just_completed_interview;
use pierre_commands::fortnight::FortnightHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::models::{GuidedFlow, OnboardingState, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::CoachesCtx;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;
mod helpers;

/// An athlete, the agent they talk to, and a conversation between them.
///
/// The agent is a real published row rather than a slug: `chat_conversations`
/// carries a foreign key to it, and the plan is stored against it, so a
/// fixture that invents an identifier would test a shape production cannot
/// produce.
async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId, String, String)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let email = format!("fortnight_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(resources.database(), &email).await?;
    let tenants = resources.common.repos.tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .map(|t| t.id)
        .expect("user should own a tenant");
    let agent = publish_catalogue_coach(
        &resources.common.repos,
        user_id,
        tenant,
        "Endurance Coach",
        "You coach endurance athletes.",
    )
    .await
    .to_string();
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "fortnight",
            "gemini-2.0-flash",
            // The agent this conversation belongs to: the command resolves
            // the plan through it, exactly as `/plan` does.
            Some(&agent),
            None,
        )
        .await?;
    Ok((resources, user_id, tenant, conversation.id, agent))
}

fn ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
    locale: &str,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/fortnight".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: locale.to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: Some(conversation_id.to_owned()),
        conversation_tenant_id: tenant_id,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

/// Save a plan through the real tool, so the rows the command reads are the
/// rows a save actually writes.
async fn save_plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    payload: Value,
) -> Result<()> {
    let executor = Arc::new(
        UniversalToolExecutor::new(Arc::<ServerContext>::clone(resources))
            .with_scopes(OAuthScope::self_grant()),
    );
    let saved = executor
        .execute_tool(UniversalRequest {
            tool_name: "save_training_plan".to_owned(),
            parameters: payload,
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        })
        .await?;
    assert!(saved.success, "plan save failed: {:?}", saved.error);
    Ok(())
}

/// The Monday `offset_weeks` from the athlete's current week.
///
/// Anchored on a Monday rather than on today, because a stored week is seven
/// days long and every day it holds must fall inside it — a week starting on
/// a Wednesday is a shape the save tool would take and no coach would write.
fn week_start(offset_weeks: i64) -> chrono::NaiveDate {
    let today = chrono::Utc::now().date_naive();
    let monday = today
        - chrono::Duration::days(i64::from(
            chrono::Datelike::weekday(&today).num_days_from_monday(),
        ));
    monday + chrono::Duration::weeks(offset_weeks)
}

fn day_of(week: chrono::NaiveDate) -> String {
    (week + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

/// A week with a real session in it. `save_training_plan` refuses an empty
/// week, and it is right to: a week with no days is not a week.
fn week(offset_weeks: i64, focus: &str, phase_index: Option<usize>) -> Value {
    let start = week_start(offset_weeks);
    let mut w = json!({
        "week_start": start.format("%Y-%m-%d").to_string(),
        "focus": focus,
        "days": [{
            "date": day_of(start),
            "sport": "run",
            "workout": "4 x 8 min at threshold",
            "duration_min": 75,
            "intensity": "threshold",
        }],
    });
    if let Some(index) = phase_index {
        w["phase_index"] = json!(index);
    }
    w
}

fn phase(offset_weeks: i64, weeks: u32) -> Value {
    json!({
        "kind": "build",
        "start": week_start(offset_weeks).format("%Y-%m-%d").to_string(),
        "weeks": weeks,
        "intent": "two hard days",
    })
}

fn plan(agent: &str, phases: &Value, weeks: &Value) -> Value {
    json!({
        // `agent_id`, not `coach_id`: an unknown key is dropped silently, and
        // a plan saved against no agent is not the plan the command reads.
        "agent_id": agent,
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": "2027-03-14", "discipline": "run_5k", "priority": "A" },
            "strategy": "polarised build",
            "phases": phases
        },
        "weeks": weeks
    })
}

#[tokio::test]
async fn with_no_plan_it_says_what_to_do_first() -> Result<()> {
    let (resources, user_id, tenant, conversation, _) = setup().await?;
    let reply = FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;

    assert!(
        reply.text.contains("no active plan"),
        "the refusal names the absence: {}",
        reply.text
    );
    assert!(
        reply.text.contains("season"),
        "and what would change it — lay out a season: {}",
        reply.text
    );
    Ok(())
}

#[tokio::test]
async fn a_plan_with_no_phases_has_nothing_to_write_against() -> Result<()> {
    let (resources, user_id, tenant, conversation, agent) = setup().await?;
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(&agent, &json!([]), &json!([week(1, "w1", None)])),
    )
    .await?;

    let reply = FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;
    assert!(
        reply.text.contains("no phases"),
        "the refusal names the phases: {}",
        reply.text
    );
    Ok(())
}

#[tokio::test]
async fn a_plan_running_out_is_written() -> Result<()> {
    let (resources, user_id, tenant, conversation, agent) = setup().await?;
    // One week only, four weeks behind the athlete: the fortnight ahead is
    // empty, which is the case this command exists for.
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(
            &agent,
            &json!([phase(-4, 12)]),
            &json!([week(-4, "old", Some(0))]),
        ),
    )
    .await?;

    let reply = FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;
    assert!(
        reply.text.contains("running out") || reply.text.contains("runs out"),
        "the go-ahead says why it is writing: {}",
        reply.text
    );
    assert!(
        reply.text.contains('2'),
        "and how many weeks it will draft: {}",
        reply.text
    );
    assert!(
        reply.text.contains("readiness"),
        "and that readiness is checked before the weeks are committed — the \
         command did not read the ladder, so it must not imply it did: {}",
        reply.text
    );
    Ok(())
}

#[tokio::test]
async fn a_plan_that_already_covers_the_fortnight_is_not_extended() -> Result<()> {
    let (resources, user_id, tenant, conversation, agent) = setup().await?;
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(
            &agent,
            &json!([phase(0, 12)]),
            &json!([week(0, "this week", Some(0)), week(1, "next week", Some(0))]),
        ),
    )
    .await?;

    let reply = FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;
    assert!(
        reply.text.contains("already covers"),
        "writing over weeks the athlete can see is how one Tuesday gets two \
         answers: {}",
        reply.text
    );
    Ok(())
}

#[tokio::test]
async fn the_refusal_reaches_the_athlete_in_their_own_language() -> Result<()> {
    let (resources, user_id, tenant, conversation, _) = setup().await?;

    for (locale, fragment) in [
        ("fr", "plan actif"),
        ("es", "plan activo"),
        ("de", "aktiven Plan"),
        ("pt", "plano ativo"),
    ] {
        let reply = FortnightHandler
            .execute(&ctx(&resources, user_id, tenant, &conversation, locale))
            .await?;
        assert!(
            reply.text.contains(fragment),
            "{locale}: expected the localised refusal, got: {}",
            reply.text
        );
        assert!(
            !reply.text.contains("commands.fortnight"),
            "{locale}: the key leaked instead of its value: {}",
            reply.text
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_go_ahead_opens_the_rail_that_keeps_its_promise() -> Result<()> {
    // The reply says two weeks are being drafted and readiness checked first.
    // Nothing on an ordinary coaching turn would do either; the rail is what
    // turns the following turns into the drafting one, so a go-ahead without
    // it is a promise the platform cannot keep.
    let (resources, user_id, tenant, conversation, agent) = setup().await?;
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(
            &agent,
            &json!([phase(-4, 12)]),
            &json!([week(-4, "old", Some(0))]),
        ),
    )
    .await?;

    FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;

    let stored = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation, &user_id.to_string(), tenant)
        .await?
        .expect("the conversation exists")
        .onboarding_state;
    let open = OnboardingState::from_column(stored.as_deref())
        .expect("the rail is open on the conversation");
    assert_eq!(
        open.flow,
        GuidedFlow::Fortnight,
        "the following turns belong to the fortnight rail: {stored:?}"
    );
    assert_eq!(
        open.turns_owned, 0,
        "and it has spent none of its budget yet: {stored:?}"
    );
    assert!(
        !just_completed_interview(stored.as_deref(), Utc::now()),
        "an OPEN rail is not a finished interview — routing it to the release \
         directive would revoke a rule the athlete never met"
    );
    Ok(())
}

#[tokio::test]
async fn an_athlete_mid_walk_is_not_promised_a_fortnight() -> Result<()> {
    // Slash commands dispatch during a guided walk, and the walk's own
    // directive owns the next turn — a brief written under it would be
    // overwritten and the fortnight never drafted. The walk must also survive:
    // answering `/fortnight` is not a reason to lose an interview in progress.
    let (resources, user_id, tenant, conversation, agent) = setup().await?;
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(
            &agent,
            &json!([phase(-4, 12)]),
            &json!([week(-4, "old", Some(0))]),
        ),
    )
    .await?;
    let walk = OnboardingState::start_now_column(GuidedFlow::Season);
    resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(&conversation, Some(&walk), tenant)
        .await?;

    let reply = FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;
    assert!(
        reply.text.contains("guided walk"),
        "the refusal names the walk and what would change it: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("Drafting"),
        "nothing is being drafted: {}",
        reply.text
    );

    let stored = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation, &user_id.to_string(), tenant)
        .await?
        .expect("the conversation exists")
        .onboarding_state;
    assert_eq!(
        OnboardingState::from_column(stored.as_deref()).map(|s| s.flow),
        Some(GuidedFlow::Season),
        "the walk the athlete is in the middle of is still running: {stored:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_turn_that_cannot_be_armed_does_not_claim_the_athlete_is_mid_walk() -> Result<()> {
    // `leave_brief` fails for four reasons and only one of them is a walk.
    // With no conversation there is no next turn to brief and no walk to
    // speak of, so the reply must say what it knows — the drafting did not
    // start — and not invent a state the athlete is not in.
    let (resources, user_id, tenant, _, agent) = setup().await?;
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(
            &agent,
            &json!([phase(-4, 12)]),
            &json!([week(-4, "old", Some(0))]),
        ),
    )
    .await?;

    // Without a conversation the agent is resolved from the athlete's
    // selection instead, so the plan is still found and the decision still
    // reaches the go-ahead — which is the arm this test is about.
    resources
        .common
        .repos
        .tenants
        .set_selected_coach(tenant, user_id, Some(&agent))
        .await?;

    let mut no_conversation = ctx(&resources, user_id, tenant, "unused", "en");
    no_conversation.conversation_id = None;
    let reply = FortnightHandler.execute(&no_conversation).await?;

    assert!(
        !reply.text.contains("guided walk"),
        "no walk is running, so the refusal must not name one: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("Drafting"),
        "and nothing is being drafted, because nothing carries the brief: {}",
        reply.text
    );
    assert!(
        reply.text.contains("haven't started"),
        "it says what it knows: {}",
        reply.text
    );
    Ok(())
}

#[tokio::test]
async fn the_newest_thing_asked_for_owns_the_next_turn() -> Result<()> {
    // A season walk has just ended and its release directive is waiting —
    // the wrap-up offered to lay out the season. Instead of answering, the
    // athlete typed `/fortnight`. The fortnight brief replaces the season's
    // offer rather than queueing behind it, because the next turn carries one
    // directive and the athlete just said which one they want.
    let (resources, user_id, tenant, conversation, agent) = setup().await?;
    save_plan(
        &resources,
        user_id,
        tenant,
        plan(
            &agent,
            &json!([phase(-4, 12)]),
            &json!([week(-4, "old", Some(0))]),
        ),
    )
    .await?;
    let retired_season = OnboardingState::start(Utc::now().to_rfc3339(), GuidedFlow::Season)
        .completed(Utc::now().to_rfc3339())
        .to_column()?;
    resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(&conversation, Some(&retired_season), tenant)
        .await?;

    let reply = FortnightHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation, "en"))
        .await?;
    assert!(
        reply.text.contains("Drafting"),
        "a retired marker is not a walk in progress, so it does not refuse: {}",
        reply.text
    );

    let stored = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation, &user_id.to_string(), tenant)
        .await?
        .expect("the conversation exists")
        .onboarding_state;
    assert_eq!(
        OnboardingState::from_column(stored.as_deref()).map(|s| s.flow),
        Some(GuidedFlow::Fortnight),
        "the fortnight rail replaced the season release: {stored:?}"
    );
    assert!(
        !just_completed_interview(stored.as_deref(), Utc::now()),
        "and the turn belongs to the rail, not to an offer the athlete walked \
         away from"
    );
    Ok(())
}

#[tokio::test]
async fn a_save_names_the_argument_keys_it_ignored() -> Result<()> {
    // The bug this closes, reproduced: `coach_id` is not a field
    // save_training_plan has, serde drops it in silence, and the plan saves
    // against no agent. The athlete then hears "no active plan to extend yet"
    // on a later turn with nothing naming the cause — which is exactly how it
    // cost several debugging steps while these tests were being written.
    let (resources, user_id, tenant, _, agent) = setup().await?;
    let executor = Arc::new(
        UniversalToolExecutor::new(Arc::<ServerContext>::clone(&resources))
            .with_scopes(OAuthScope::self_grant()),
    );
    let mut payload = plan(
        &agent,
        &json!([phase(-4, 12)]),
        &json!([week(-4, "old", Some(0))]),
    );
    // The wrong key, beside the right one, plus a provider-envelope-shaped
    // stray so the report is a list rather than a lucky single.
    payload["coach_id"] = json!(agent);
    payload["parameters"] = json!({"agent_id": agent});

    let saved = executor
        .execute_tool(UniversalRequest {
            tool_name: "save_training_plan".to_owned(),
            parameters: payload,
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        })
        .await?;
    assert!(saved.success, "the save still succeeds: {:?}", saved.error);

    let reported = saved
        .result
        .as_ref()
        .and_then(|r| r.get("ignored_arguments"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(
        reported,
        vec!["coach_id".to_owned(), "parameters".to_owned()],
        "both dropped keys are named, sorted, on the turn they were sent"
    );
    Ok(())
}

#[tokio::test]
async fn a_well_formed_save_reports_no_ignored_arguments() -> Result<()> {
    // The control. Without this, the assertion above passes just as well when
    // the reporter names every key it was given.
    let (resources, user_id, tenant, _, agent) = setup().await?;
    let executor = Arc::new(
        UniversalToolExecutor::new(Arc::<ServerContext>::clone(&resources))
            .with_scopes(OAuthScope::self_grant()),
    );
    let saved = executor
        .execute_tool(UniversalRequest {
            tool_name: "save_training_plan".to_owned(),
            parameters: plan(
                &agent,
                &json!([phase(-4, 12)]),
                &json!([week(-4, "old", Some(0))]),
            ),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        })
        .await?;
    assert!(saved.success, "plan save failed: {:?}", saved.error);
    assert!(
        saved
            .result
            .as_ref()
            .and_then(|r| r.get("ignored_arguments"))
            .is_none(),
        "an all-known payload reports nothing and the field leaves the wire"
    );
    Ok(())
}
