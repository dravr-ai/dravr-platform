// ABOUTME: End-to-end coach checklist (carnet#1529): GET /api/me/coach-setup ticks the seat, the athlete invited and the platform
// ABOUTME: Pins that each row reads server state — coach access, a member who joined, a connection of the coach's own

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A user who coaches others without coach access lands on a coach-shaped
//! Home whose checklist is this route. These tests drive it from the state
//! each row names: a fresh coach has nothing done and the seat current; a
//! member who joined one of their groups ticks the invite; a TrainingPeaks or
//! Intervals.icu connection of their own ticks the platform, a synthetic seed
//! does not; a grant ticks the seat. A group the coach owns that another
//! coach runs is not theirs, and is never listed.

use crate::common;
use crate::helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_access::CoachAccessRoutes;
use pierre_test_support::delegation::{
    add_group_member, create_coached_group, create_coachless_group, create_group_agent,
};
use serde_json::Value;
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

/// A signed-in user: the bearer header, the user id, its tenant.
struct Session {
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
}

/// Create an active user named `name` owning a tenant, and sign it in.
async fn session(resources: &Arc<ServerContext>, email: &str, name: &str) -> Result<Session> {
    let mut user = User::new(
        email.to_owned(),
        "unused-hash".to_owned(),
        Some(name.to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;

    let tenant_id = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await?;

    let token = generate_test_token(resources, &user).await;
    Ok(Session {
        auth: format!("Bearer {token}"),
        user_id,
        tenant_id,
    })
}

/// The caller's checklist.
async fn checklist(resources: &Arc<ServerContext>, auth: &str) -> Value {
    let response = AxumTestRequest::get("/api/me/coach-setup")
        .header("Authorization", auth)
        .send(CoachAccessRoutes::routes(Arc::clone(resources)))
        .await;
    assert_eq!(response.status(), 200, "the checklist reads");
    response.json()
}

/// The row `id` of a checklist body.
fn row<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("a {id} row in {body}"))
}

#[tokio::test]
#[serial]
async fn a_fresh_coach_has_nothing_done_and_the_seat_current() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let coach = session(&resources, "fresh-coach@example.com", "Marc").await?;

    let body = checklist(&resources, &coach.auth).await;
    let ids: Vec<&str> = body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["seat", "invite", "platform"], "{body}");
    assert_eq!(body["done_count"], 0);
    assert_eq!(body["total"], 3);
    assert_eq!(row(&body, "seat")["current"], true);
    assert_eq!(row(&body, "invite")["current"], false);
    assert_eq!(body["groups"], Value::Array(Vec::new()));
    assert_eq!(body["athlete"], Value::Null);
    assert_eq!(body["platform"], Value::Null);
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_member_who_joined_the_coachs_group_ticks_the_invite() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "invite-coach@example.com", "Marc").await?;
    let agent = create_group_agent(repos, coach.user_id, coach.tenant_id).await?;
    let group = create_coachless_group(
        repos,
        coach.user_id,
        coach.tenant_id,
        &agent,
        "Léa · Marathon",
    )
    .await?;

    let empty = checklist(&resources, &coach.auth).await;
    assert_eq!(
        row(&empty, "invite")["done"],
        false,
        "a group alone is no athlete"
    );
    assert_eq!(empty["groups"][0]["id"], group.to_string());
    assert_eq!(empty["groups"][0]["name"], "Léa · Marathon");

    // The coach's own membership is never the athlete.
    add_group_member(repos, group, coach.user_id, coach.tenant_id).await?;
    let own = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&own, "invite")["done"], false, "{own}");

    let athlete = session(&resources, "lea@example.com", "Léa Gagnon").await?;
    add_group_member(repos, group, athlete.user_id, athlete.tenant_id).await?;

    let body = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&body, "invite")["done"], true, "{body}");
    assert_eq!(body["done_count"], 1);
    assert_eq!(
        row(&body, "seat")["current"],
        true,
        "the seat is still the step to take"
    );
    assert_eq!(body["athlete"]["name"], "Léa Gagnon");
    assert_eq!(body["athlete"]["group_id"], group.to_string());
    assert!(body["athlete"]["joined_at"].is_string(), "{body}");
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_group_another_coach_runs_is_not_the_owners_to_coach() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let owner = session(&resources, "owner@example.com", "Marc").await?;
    let other = session(&resources, "other-coach@example.com", "Julie").await?;
    let agent = create_group_agent(repos, owner.user_id, owner.tenant_id).await?;
    let group = create_coached_group(
        repos,
        owner.user_id,
        owner.tenant_id,
        &agent,
        "Coached elsewhere",
        other.user_id,
    )
    .await?;
    let athlete = session(&resources, "member@example.com", "Félix").await?;
    add_group_member(repos, group, athlete.user_id, athlete.tenant_id).await?;

    let body = checklist(&resources, &owner.auth).await;
    assert_eq!(body["groups"], Value::Array(Vec::new()), "{body}");
    assert_eq!(row(&body, "invite")["done"], false);

    // The coach who runs it does have it, and its member.
    let theirs = checklist(&resources, &other.auth).await;
    assert_eq!(theirs["groups"][0]["id"], group.to_string(), "{theirs}");
    assert_eq!(row(&theirs, "invite")["done"], true);
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_coaching_platform_of_the_coachs_own_ticks_the_platform() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "platform-coach@example.com", "Marc").await?;

    // A seeded connection is no account the coach linked.
    repos
        .provider_connections
        .register_connection(
            coach.user_id,
            coach.tenant_id,
            "intervals_icu",
            &ConnectionType::Synthetic,
            None,
        )
        .await?;
    let seeded = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&seeded, "platform")["done"], false, "{seeded}");
    assert_eq!(seeded["platform"], Value::Null);

    // Another provider is not a coaching platform.
    repos
        .provider_connections
        .register_connection(
            coach.user_id,
            coach.tenant_id,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await?;
    let strava = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&strava, "platform")["done"], false, "{strava}");

    repos
        .provider_connections
        .register_connection(
            coach.user_id,
            coach.tenant_id,
            "sciotte_trainingpeaks",
            &ConnectionType::Manual,
            None,
        )
        .await?;
    let body = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&body, "platform")["done"], true, "{body}");
    assert_eq!(body["platform"], "sciotte_trainingpeaks");

    // Both linked, the checklist names TrainingPeaks, the platform it offers
    // first — not the one linked last.
    repos
        .provider_connections
        .register_connection(
            coach.user_id,
            coach.tenant_id,
            "intervals_icu",
            &ConnectionType::OAuth,
            None,
        )
        .await?;
    let both = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&both, "platform")["done"], true, "{both}");
    assert_eq!(both["platform"], "sciotte_trainingpeaks", "{both}");
    Ok(())
}

#[tokio::test]
#[serial]
async fn coach_access_ticks_the_seat_and_keeps_the_groups_it_attached() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "granted-coach@example.com", "Marc").await?;
    let agent = create_group_agent(repos, coach.user_id, coach.tenant_id).await?;
    let group = create_coached_group(
        repos,
        coach.user_id,
        coach.tenant_id,
        &agent,
        "Léa · Marathon",
        coach.user_id,
    )
    .await?;
    repos.users.set_manages_roster(coach.user_id, true).await?;

    let body = checklist(&resources, &coach.auth).await;
    assert_eq!(row(&body, "seat")["done"], true, "{body}");
    assert_eq!(row(&body, "seat")["current"], false);
    assert_eq!(row(&body, "invite")["current"], true);
    assert_eq!(
        body["groups"][0]["id"],
        group.to_string(),
        "the group they coach stays"
    );
    Ok(())
}
