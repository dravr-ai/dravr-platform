// ABOUTME: HTTP tests for the lead's first welcome the team step writes — synced history read back, the goal and the why quoted
// ABOUTME: Pins carnet#1528: figures from the durable activity cache with its stamp, none without data or once the first-sync post spoke, no question
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use crate::helpers::agent_fixtures::publish_catalogue_agent;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::json;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::messaging_strings::{
    format_template, KEY_AGENT_WELCOME_DISCOVER_LINE, KEY_AGENT_WELCOME_FIGURES_READ,
    KEY_AGENT_WELCOME_FIGURES_READ_ONE, KEY_AGENT_WELCOME_GOAL_WHY, KEY_AGENT_WELCOME_TEAM_SOLO,
};
use pierre_core::models::{
    Activity, ActivityBuilder, AddMessageParams, MessageRecord, PostPurpose, SportType, TenantId,
};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_services::about_you::{persist_about_you, AboutYouAnswers};
use pierre_services::locale::resolve_user_locale;
use pierre_services::personal_thread::{
    find_personal_thread, resolve_personal_thread, PersonalForge,
};
use uuid::Uuid;

/// An athlete who holds no agent yet, with a token for their tenant.
async fn athlete() -> (Arc<ServerContext>, axum::Router, String, Uuid, TenantId) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = OnboardingRoutes::routes(Arc::clone(&resources));
    (resources, router, token, user_id, tenant)
}

/// The team step's Skip, naming `lead` as the athlete's lead: the write that
/// opens the personal thread on the lead's welcome.
async fn skip_to(
    resources: &ServerContext,
    router: &axum::Router,
    token: &str,
    user_id: Uuid,
    tenant: TenantId,
) {
    let lead = publish_catalogue_agent(
        resources.agent.database.repositories(),
        user_id,
        tenant,
        "Tempo Agent",
        "Test.",
    )
    .await
    .to_string();
    let status = AxumTestRequest::put("/api/me/onboarding/steps/coach_proposal")
        .header("Authorization", token)
        .json(&json!({ "status": "skipped", "agent_id": lead, "team_agent_ids": [lead] }))
        .send(router.clone())
        .await
        .status_code();
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// The welcome row of the athlete's personal thread.
async fn welcome_row(resources: &ServerContext, user_id: Uuid, tenant: TenantId) -> MessageRecord {
    let thread = find_personal_thread(&resources.common.repos, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .expect("the team step forges the personal thread");
    resources
        .agent
        .database
        .repositories()
        .chat
        .get_messages(&thread.id, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .into_iter()
        .find(MessageRecord::is_agent_welcome)
        .expect("the lead's welcome opens the thread")
}

/// The catalogue text for `key` in the athlete's own locale.
async fn said(resources: &ServerContext, user_id: Uuid, key: &str) -> String {
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await;
    resources.mcp.messaging_strings_registry.get(key, &locale)
}

fn session(id: &str, sport: SportType, days_ago: i64, seconds: u64, metres: f64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        id.to_owned(),
        sport,
        Utc::now() - Duration::days(days_ago),
        seconds,
        "strava".to_owned(),
    )
    .distance_meters(metres)
    .build()
}

/// Store `rows` in the athlete's durable activity cache, as a sync lands them.
async fn synced(resources: &ServerContext, user_id: Uuid, tenant: TenantId, rows: &[Activity]) {
    resources
        .agent
        .database
        .repositories()
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", rows)
        .await
        .unwrap();
}

/// carnet#1528: the welcome Home opens on reads the synced history back — the
/// days it read, the sessions and their sports — and is stamped with what it
/// read: Strava data never leaves Dravr's own apps.
#[tokio::test]
async fn the_leads_welcome_reads_back_the_synced_history() {
    let (resources, router, token, user_id, tenant) = athlete().await;
    synced(
        &resources,
        user_id,
        tenant,
        &[
            session("run-1", SportType::Run, 19, 1_800, 5_000.0),
            session("ride-1", SportType::Ride, 12, 3_600, 30_000.0),
            session("run-2", SportType::Run, 6, 2_400, 8_000.0),
            session("run-3", SportType::Run, 1, 6_734, 21_400.0),
        ],
    )
    .await;

    skip_to(&resources, &router, &token, user_id, tenant).await;
    let row = welcome_row(&resources, user_id, tenant).await;

    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await;
    let strings = &resources.mcp.messaging_strings_registry;
    let mix = format!(
        "{} 3, {} 1",
        strings.get("app.sportRun", &locale),
        strings.get("app.sportRide", &locale)
    );
    let read = format_template(
        &said(&resources, user_id, KEY_AGENT_WELCOME_FIGURES_READ).await,
        &["20", "4", &mix],
    );
    assert!(row.content.contains(&read), "{}", row.content);
    // The team is still introduced.
    let solo = said(&resources, user_id, KEY_AGENT_WELCOME_TEAM_SOLO).await;
    assert!(row.content.contains(&solo), "{}", row.content);
    assert_eq!(
        row.transport_policy,
        TransportPolicy::FirstPartyOnly,
        "a welcome built on Strava activities is stamped first-party only"
    );
}

/// The figure line's own words, before its first placeholder: what a welcome
/// that read the history back says, whatever it counted.
async fn figures_opening(resources: &ServerContext, user_id: Uuid) -> String {
    let read = said(resources, user_id, KEY_AGENT_WELCOME_FIGURES_READ_ONE).await;
    let opening = read
        .split("{0}")
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    assert!(!opening.is_empty(), "the catalogue holds the figure line");
    opening
}

/// With nothing synced the welcome is the plain greeting: no figure line, and
/// nothing asked — it ends on Discover, as the team introduction does.
#[tokio::test]
async fn a_welcome_with_no_history_says_no_figures() {
    let (resources, router, token, user_id, tenant) = athlete().await;
    skip_to(&resources, &router, &token, user_id, tenant).await;
    let row = welcome_row(&resources, user_id, tenant).await;

    let opening = figures_opening(&resources, user_id).await;
    assert!(!row.content.contains(&opening), "{}", row.content);
    let discover = said(&resources, user_id, KEY_AGENT_WELCOME_DISCOVER_LINE).await;
    assert!(
        row.content.trim_end().ends_with(discover.trim()),
        "the welcome asks no question: {}",
        row.content
    );
    assert_eq!(row.transport_policy, TransportPolicy::AnyTransport);
}

/// carnet#831's first post spoke about the athlete's data in their personal
/// thread: it is a model reply there, and the first-sync ledger records that
/// it landed.
async fn first_sync_posted(resources: &ServerContext, user_id: Uuid, tenant: TenantId) {
    let repos = &resources.common.repos;
    let user = user_id.to_string();
    let thread = resolve_personal_thread(
        repos,
        &user,
        tenant,
        PersonalForge {
            title_fallback: "Personal",
            guided_flow: false,
            llm_chain: &resources.common.config.llm_chain,
        },
    )
    .await
    .unwrap()
    .conversation;
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: tenant,
            conversation_id: &thread.id,
            user_id: &user,
            role: "assistant",
            content: "Your Strava history just landed: four sessions in three weeks, mostly runs.",
            token_count: None,
            finish_reason: Some("stop"),
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::FirstPartyOnly,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();
    let purpose = PostPurpose::FirstSync.as_str();
    let now = Utc::now();
    let lease = repos
        .platform_post_claims
        .claim_platform_post_once(
            tenant,
            user_id,
            purpose,
            PostPurpose::NO_SUBJECT,
            now - Duration::minutes(15),
            now,
        )
        .await
        .unwrap()
        .expect("the first-sync claim is free");
    assert!(repos
        .platform_post_claims
        .mark_platform_post_posted(
            tenant,
            user_id,
            purpose,
            PostPurpose::NO_SUBJECT,
            &lease,
            Utc::now()
        )
        .await
        .unwrap());
}

/// Phil, 2026-11-14: once carnet#831's first post spoke about the synced
/// history in the personal thread, the welcome leaves the figures to it — and
/// says everything else it would have said.
#[tokio::test]
async fn the_welcome_leaves_the_figures_to_a_first_sync_post_that_already_spoke() {
    let (resources, router, token, user_id, tenant) = athlete().await;
    synced(
        &resources,
        user_id,
        tenant,
        &[
            session("run-1", SportType::Run, 19, 1_800, 5_000.0),
            session("run-2", SportType::Run, 1, 6_734, 21_400.0),
        ],
    )
    .await;
    first_sync_posted(&resources, user_id, tenant).await;

    skip_to(&resources, &router, &token, user_id, tenant).await;
    let row = welcome_row(&resources, user_id, tenant).await;

    let opening = figures_opening(&resources, user_id).await;
    assert!(!row.content.contains(&opening), "{}", row.content);
    let solo = said(&resources, user_id, KEY_AGENT_WELCOME_TEAM_SOLO).await;
    let discover = said(&resources, user_id, KEY_AGENT_WELCOME_DISCOVER_LINE).await;
    assert!(row.content.contains(&solo), "{}", row.content);
    assert!(row.content.contains(&discover), "{}", row.content);
}

/// The goal and the why the athlete gave About you are quoted back.
#[tokio::test]
async fn the_welcome_quotes_the_goal_and_the_why() {
    let (resources, router, token, user_id, tenant) = athlete().await;
    persist_about_you(
        resources.agent.database.repositories().memory.as_ref(),
        tenant,
        &user_id.to_string(),
        &AboutYouAnswers {
            goal: Some("Montreal marathon under 3:45".to_owned()),
            north_star: Some("to run the race my father ran".to_owned()),
            ..AboutYouAnswers::default()
        },
    )
    .await
    .unwrap();

    skip_to(&resources, &router, &token, user_id, tenant).await;
    let row = welcome_row(&resources, user_id, tenant).await;

    let quoted = format_template(
        &said(&resources, user_id, KEY_AGENT_WELCOME_GOAL_WHY).await,
        &[
            "Montreal marathon under 3:45",
            "to run the race my father ran",
        ],
    );
    assert!(row.content.contains(&quoted), "{}", row.content);
}
