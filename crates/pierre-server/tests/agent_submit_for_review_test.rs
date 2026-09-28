// ABOUTME: POST /api/agents/{id}/submit puts the author's own agent in the admin Store review queue
// ABOUTME: Pins the owner-only refusal (403), the queue entry, and that an admin approval publishes it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use chrono::Utc;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::{build_agents_admin_router, build_agents_router};
use serde_json::Value;
use serial_test::serial;
use uuid::Uuid;

/// The user and admin agent routers, as the server mounts them. A router is
/// consumed by each `send`, so build one per request.
fn router(resources: &Arc<ServerContext>) -> Router {
    Router::new()
        .merge(build_agents_router::<ServerContext>().with_state(Arc::clone(resources)))
        .nest(
            "/api/admin",
            build_agents_admin_router::<ServerContext>().with_state(Arc::clone(resources)),
        )
}

/// An active user in `tenant`, returning it and `"Bearer <jwt>"`.
async fn member(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    admin: bool,
) -> Result<(User, String)> {
    let email = format!("submit-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, "hash".to_owned(), Some("Member".to_owned()));
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    if admin {
        user.is_admin = true;
        user.role = UserRole::Admin;
    }
    resources.common.repos.users.create(&user).await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user.id, tenant)
        .await?;
    let token = generate_test_token(resources, &user).await;
    Ok((user, format!("Bearer {token}")))
}

async fn tenant_owned_by(resources: &Arc<ServerContext>) -> Result<(TenantId, User, String)> {
    let email = format!("owner-{}@example.com", Uuid::new_v4());
    let mut owner = User::new(email, "hash".to_owned(), Some("Author".to_owned()));
    owner.user_status = UserStatus::Active;
    resources.common.repos.users.create(&owner).await?;
    let tenant_id = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Submit Tenant".to_owned(),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(owner.id, tenant_id)
        .await?;
    let token = generate_test_token(resources, &owner).await;
    Ok((tenant_id, owner, format!("Bearer {token}")))
}

#[tokio::test]
#[serial]
async fn the_author_submits_an_admin_approves_and_it_is_published() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (tenant, author, author_bearer) = tenant_owned_by(&resources).await?;
    let (_other, other_bearer) = member(&resources, tenant, false).await?;
    let (_admin, admin_bearer) = member(&resources, tenant, true).await?;

    let agent = resources
        .common
        .repos
        .agents
        .create(
            author.id,
            tenant,
            &CreateAgentRequest {
                title: "Hill Specialist".to_owned(),
                description: Some("Climbs, mostly".to_owned()),
                system_prompt: "You coach hills.".to_owned(),
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
        .await?;
    let id = agent.id.to_string();
    let submit = format!("/api/agents/{id}/submit");

    // Someone else in the tenant may see the agent but not submit it.
    let refused = AxumTestRequest::post(&submit)
        .header("authorization", &other_bearer)
        .send(router(&resources))
        .await;
    assert_eq!(refused.status(), 403, "{}", refused.text());
    let listing = resources
        .common
        .repos
        .store_listings
        .get_listing(&id)
        .await?;
    assert!(listing.is_none(), "a refused submit creates no listing");

    // The author's submit lands in the review queue.
    let submitted = AxumTestRequest::post(&submit)
        .header("authorization", &author_bearer)
        .send(router(&resources))
        .await;
    assert_eq!(submitted.status(), 200, "{}", submitted.text());
    let body: Value = serde_json::from_str(&submitted.text())?;
    assert_eq!(body["agent_id"], id);
    assert_eq!(body["publish_status"], "pending_review");
    assert!(body["review_submitted_at"].is_string(), "{body}");

    let queue = AxumTestRequest::get("/api/admin/store/review-queue")
        .header("authorization", &admin_bearer)
        .send(router(&resources))
        .await;
    assert_eq!(queue.status(), 200, "{}", queue.text());
    let queue: Value = serde_json::from_str(&queue.text())?;
    assert!(
        queue["agents"]
            .as_array()
            .is_some_and(|agents| agents.iter().any(|a| a["id"] == id)),
        "the submitted agent waits in the review queue: {queue}"
    );

    // An admin approval publishes it.
    let approved = AxumTestRequest::post(&format!("/api/admin/store/agents/{id}/approve"))
        .header("authorization", &admin_bearer)
        .send(router(&resources))
        .await;
    assert_eq!(approved.status(), 200, "{}", approved.text());
    let listing = resources
        .common
        .repos
        .store_listings
        .get_listing(&id)
        .await?
        .expect("the approved agent has a listing");
    assert_eq!(listing.publish_status.as_str(), "published");
    assert!(listing.published_at.is_some());
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_unknown_agent_is_not_found() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_tenant, _author, author_bearer) = tenant_owned_by(&resources).await?;
    let response = AxumTestRequest::post(&format!("/api/agents/{}/submit", Uuid::new_v4()))
        .header("authorization", &author_bearer)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 404, "{}", response.text());
    Ok(())
}
