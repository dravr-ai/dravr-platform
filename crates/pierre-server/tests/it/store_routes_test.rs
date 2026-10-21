// ABOUTME: Integration tests for Agent Store REST API routes
// ABOUTME: Tests browsing, searching, installing, and uninstalling agents from the Store
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use common::{
    create_test_server_resources, create_test_user, create_test_user_with_email,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_core::models::{CreateGroupRequest, TenantId};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::database::agents::{
    AgentCategory, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest, PublishStatus,
};
use pierre_database::database::Agent;
use pierre_groups::strategies::tier::EnterpriseTierStrategy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::build_store_router;
use pierre_routes_agents::store::{
    BrowseAgentsResponse, InstallAgentResponse, InstallationsResponse, SearchAgentsResponse,
    StoreAgentDetail, UninstallAgentResponse,
};
use std::sync::Arc;
use uuid::Uuid;

use axum::http::StatusCode;
use chrono::{DateTime, Duration};
use serial_test::serial;

/// Pins a listing's `published_at`, so cursor pagination can be driven through ties.
const SET_PUBLISHED_AT: &str = "UPDATE store_listings SET published_at = $1 WHERE agent_id = $2";

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_test_environment() -> (axum::Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    // Generate a JWT token for the user
    let token = generate_test_token(&resources, &user).await;

    // Create the store router
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    (router, format!("Bearer {token}"))
}

/// Create a published agent in the Store for testing
async fn create_published_agent(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    category: AgentCategory,
) -> Agent {
    let tags = vec!["test".to_owned(), category.as_str().to_owned()];
    create_published_agent_tagged(resources, user_id, tenant_id, title, category, tags).await
}

/// [`create_published_agent`] with the agent's tags given.
async fn create_published_agent_tagged(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    category: AgentCategory,
    tags: Vec<String>,
) -> Agent {
    let agents_manager = &resources.common.repos.agents;
    let store_listings_manager = &resources.common.repos.store_listings;

    // Create as system agent first (can set visibility)
    let system_request = CreateSystemAgentRequest {
        title: title.to_owned(),
        description: Some(format!("Description for {title}")),
        system_prompt: format!("You are a {title} coach."),
        category,
        tags,
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec!["Sample prompt 1".to_owned()],
    };

    let agent = agents_manager
        .create_system_agent(user_id, tenant_id, &system_request)
        .await
        .unwrap();

    // Submit for review and approve to publish
    // Note: We use the same user_id as admin to avoid FK constraint issues in tests
    store_listings_manager
        .submit_for_review(&agent.id.to_string(), user_id, tenant_id)
        .await
        .unwrap();

    let agent_with_listing = store_listings_manager
        .approve_agent(&agent.id.to_string(), tenant_id, Some(user_id))
        .await
        .unwrap();

    agent_with_listing.agent
}

/// The tenant `user_id` owns, which their test token carries as active.
async fn tenant_of(resources: &ServerContext, user_id: Uuid) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id
}

/// A listing's install counter, read from the repository.
async fn listing_install_count(resources: &ServerContext, agent_id: &str) -> u32 {
    resources
        .common
        .repos
        .store_listings
        .get_listing(agent_id)
        .await
        .unwrap()
        .expect("the agent is listed")
        .install_count
}

/// An athlete-authored (not system) agent approved into the Store.
async fn create_user_published_agent(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
) -> Agent {
    let request: CreateAgentRequest = serde_json::from_value(serde_json::json!({
        "title": title,
        "system_prompt": format!("You are {title}."),
        "category": "training",
        "tags": ["test"],
    }))
    .unwrap();
    let agent = resources
        .common
        .repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    let id = agent.id.to_string();
    let listings = &resources.common.repos.store_listings;
    listings
        .submit_for_review(&id, user_id, tenant_id)
        .await
        .unwrap();
    listings
        .approve_agent(&id, tenant_id, Some(user_id))
        .await
        .unwrap()
        .agent
}

// ============================================================================
// Browse Store Tests
// ============================================================================

#[tokio::test]
async fn test_browse_store_empty() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/store/agents")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: BrowseAgentsResponse = response.json();
    assert!(result.agents.is_empty());
    assert!(!result.has_more);
    assert!(result.next_cursor.is_none());
    assert_ne!(result.metadata.timestamp, "");
    assert_eq!(result.metadata.api_version, "1.0");
}

#[tokio::test]
async fn test_browse_store_with_published_agents() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    // Create published agents
    create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Marathon Agent",
        AgentCategory::Training,
    )
    .await;
    create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Nutrition Guide",
        AgentCategory::Nutrition,
    )
    .await;

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::get("/api/store/agents")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: BrowseAgentsResponse = response.json();
    assert_eq!(result.agents.len(), 2);
    assert!(!result.has_more);
    assert!(result.next_cursor.is_none());
}

#[tokio::test]
async fn test_browse_store_with_category_filter() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Training Coach",
        AgentCategory::Training,
    )
    .await;
    create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Nutrition Coach",
        AgentCategory::Nutrition,
    )
    .await;

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::get("/api/store/agents?category=training")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: BrowseAgentsResponse = response.json();
    assert_eq!(result.agents.len(), 1);
    assert_eq!(result.agents[0].category, AgentCategory::Training);
    assert!(!result.has_more);
}

#[tokio::test]
#[serial]
async fn test_browse_store_with_cursor_pagination() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    // No inter-seed delay on purpose: the cursor round-trips published_at at
    // full precision (dravr-carnet#31), so agents landing in the same
    // millisecond paginate cleanly. The collision case is exercised
    // deliberately in cursor_pagination_survives_same_millisecond_published_at.
    for i in 1..=5 {
        create_published_agent(
            &resources,
            user_id,
            tenant_id,
            &format!("Coach {i}"),
            AgentCategory::Training,
        )
        .await;
    }

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Get first page
    let response = AxumTestRequest::get("/api/store/agents?limit=2")
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page1: BrowseAgentsResponse = response.json();
    assert_eq!(page1.agents.len(), 2);
    assert!(page1.has_more);
    assert!(page1.next_cursor.is_some());

    // Get second page using cursor
    let cursor = page1.next_cursor.unwrap();
    let response = AxumTestRequest::get(&format!("/api/store/agents?limit=2&cursor={cursor}"))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page2: BrowseAgentsResponse = response.json();
    assert_eq!(page2.agents.len(), 2);
    assert!(page2.has_more);

    // Ensure no duplicate agents between pages
    let page1_ids: Vec<_> = page1.agents.iter().map(|c| &c.id).collect();
    let page2_ids: Vec<_> = page2.agents.iter().map(|c| &c.id).collect();
    for id in &page2_ids {
        assert!(
            !page1_ids.contains(id),
            "Cursor pagination returned duplicate coach"
        );
    }

    // Get third page (should have only 1 agent)
    let cursor = page2.next_cursor.unwrap();
    let response = AxumTestRequest::get(&format!("/api/store/agents?limit=2&cursor={cursor}"))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page3: BrowseAgentsResponse = response.json();
    assert_eq!(page3.agents.len(), 1);
    assert!(!page3.has_more);
    assert!(page3.next_cursor.is_none());
}

/// The collision the millisecond-precision cursor could not represent
/// (dravr-carnet#31): listings published within one millisecond — including
/// two at the exact same instant — must paginate with no duplicate and no
/// skipped agent, the same-instant pair resolved by the id tiebreaker. Under
/// the old integer-millis cursor the boundary row matched neither the `<` nor
/// the `=` branch of the keyset predicate, so pages repeated or dropped rows
/// depending on where truncation landed.
#[tokio::test]
#[serial]
async fn cursor_pagination_survives_same_millisecond_published_at() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let mut agent_ids = Vec::new();
    for i in 1..=5 {
        let agent = create_published_agent(
            &resources,
            user_id,
            tenant_id,
            &format!("Collision Coach {i}"),
            AgentCategory::Training,
        )
        .await;
        agent_ids.push(agent.id.to_string());
    }

    // Pin every published_at into one millisecond, microseconds apart — and
    // give the first two the exact same instant. Written as to_rfc3339(), the
    // same format the production publish path uses.
    let base = DateTime::from_timestamp_micros(1_755_432_000_000_100).unwrap();
    let micro_offsets: [i64; 5] = [0, 0, 100, 200, 300];
    for (agent_id, offset) in agent_ids.iter().zip(micro_offsets) {
        let ts = base + Duration::microseconds(offset);
        match resources.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SET_PUBLISHED_AT)
                    .bind(ts.to_rfc3339())
                    .bind(agent_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            // `published_at` is a `timestamptz` column on PostgreSQL.
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SET_PUBLISHED_AT)
                    .bind(ts)
                    .bind(agent_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Walk the whole store in pages of 2 and collect every id.
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = cursor.as_ref().map_or_else(
            || "/api/store/agents?limit=2".to_owned(),
            |c| format!("/api/store/agents?limit=2&cursor={c}"),
        );
        let response = AxumTestRequest::get(&uri)
            .header("authorization", &auth_token)
            .send(router.clone())
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let page: BrowseAgentsResponse = response.json();
        for agent in &page.agents {
            let id = agent.id.to_string();
            assert!(
                !seen.contains(&id),
                "cursor pagination returned duplicate coach {id}"
            );
            seen.push(id);
        }
        if !page.has_more {
            break;
        }
        cursor = Some(page.next_cursor.expect("has_more implies a next cursor"));
    }

    assert_eq!(
        seen.len(),
        5,
        "every seeded coach must appear exactly once across pages"
    );
    for agent_id in &agent_ids {
        assert!(
            seen.contains(agent_id),
            "coach {agent_id} was skipped by the page boundary"
        );
    }
}

#[tokio::test]
#[serial]
async fn test_cursor_pagination_with_popular_sort() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    // Create agents with different install counts
    let store_listings_manager = &resources.common.repos.store_listings;

    for i in 1..=5 {
        let agent = create_published_agent(
            &resources,
            user_id,
            tenant_id,
            &format!("Popular Coach {i}"),
            AgentCategory::Training,
        )
        .await;

        // Give each agent a different install count
        for _ in 0..(6 - i) {
            store_listings_manager
                .increment_install_count(&agent.id.to_string())
                .await
                .unwrap();
        }
    }

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Get first page sorted by popular
    let response = AxumTestRequest::get("/api/store/agents?limit=2&sort_by=popular")
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page1: BrowseAgentsResponse = response.json();
    assert_eq!(page1.agents.len(), 2);
    assert!(page1.has_more);

    // Most popular should be first (highest install count)
    assert!(
        page1.agents[0].install_count >= page1.agents[1].install_count,
        "Coaches should be sorted by popularity"
    );

    // Get second page using cursor
    let cursor = page1.next_cursor.unwrap();
    let response = AxumTestRequest::get(&format!(
        "/api/store/agents?limit=2&sort_by=popular&cursor={cursor}"
    ))
    .header("authorization", &auth_token)
    .send(router)
    .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page2: BrowseAgentsResponse = response.json();
    assert_eq!(page2.agents.len(), 2);

    // Second page agents should have lower install counts than first page
    assert!(
        page1.agents[1].install_count >= page2.agents[0].install_count,
        "Second page should have lower popularity than first page"
    );
}

#[tokio::test]
#[serial]
async fn test_cursor_pagination_with_title_sort() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    // Create agents with alphabetically ordered names
    let titles = ["Alpha Coach", "Beta Coach", "Gamma Coach", "Delta Coach"];
    for title in titles {
        create_published_agent(
            &resources,
            user_id,
            tenant_id,
            title,
            AgentCategory::Training,
        )
        .await;
    }

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Get first page sorted by title
    let response = AxumTestRequest::get("/api/store/agents?limit=2&sort_by=title")
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page1: BrowseAgentsResponse = response.json();
    assert_eq!(page1.agents.len(), 2);
    assert!(page1.has_more);

    // First agent should be alphabetically first
    assert_eq!(page1.agents[0].title, "Alpha Coach");
    assert_eq!(page1.agents[1].title, "Beta Coach");

    // Get second page using cursor
    let cursor = page1.next_cursor.unwrap();
    let response = AxumTestRequest::get(&format!(
        "/api/store/agents?limit=2&sort_by=title&cursor={cursor}"
    ))
    .header("authorization", &auth_token)
    .send(router)
    .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let page2: BrowseAgentsResponse = response.json();
    assert_eq!(page2.agents.len(), 2);

    // Second page should continue alphabetically (Delta, Gamma)
    assert_eq!(page2.agents[0].title, "Delta Coach");
    assert_eq!(page2.agents[1].title, "Gamma Coach");
}

#[tokio::test]
#[serial]
async fn test_cursor_invalid_for_different_sort_order() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    for i in 1..=3 {
        create_published_agent(
            &resources,
            user_id,
            tenant_id,
            &format!("Coach {i}"),
            AgentCategory::Training,
        )
        .await;
    }

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Get cursor from newest sort
    let response = AxumTestRequest::get("/api/store/agents?limit=1&sort_by=newest")
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    let page: BrowseAgentsResponse = response.json();
    let newest_cursor = page.next_cursor.unwrap();

    // Try to use newest cursor with popular sort - should fail
    let response = AxumTestRequest::get(&format!(
        "/api/store/agents?limit=1&sort_by=popular&cursor={newest_cursor}"
    ))
    .header("authorization", &auth_token)
    .send(router)
    .await;

    // Should return a bad request error due to cursor mismatch
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_browse_store_sort_by_popular() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let _coach1 = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Less Popular",
        AgentCategory::Training,
    )
    .await;
    let coach2 = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "More Popular",
        AgentCategory::Training,
    )
    .await;

    // Simulate installs to make coach2 more popular
    let store_listings_manager = &resources.common.repos.store_listings;
    store_listings_manager
        .increment_install_count(&coach2.id.to_string())
        .await
        .unwrap();
    store_listings_manager
        .increment_install_count(&coach2.id.to_string())
        .await
        .unwrap();

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::get("/api/store/agents?sort_by=popular")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let result: BrowseAgentsResponse = response.json();
    assert_eq!(result.agents[0].title, "More Popular");
}

#[tokio::test]
async fn test_browse_store_unauthorized() {
    let (router, _) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/store/agents").send(router).await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Get Agent Detail Tests
// ============================================================================

#[tokio::test]
async fn test_get_agent_detail() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let agent = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Detail Test Coach",
        AgentCategory::Training,
    )
    .await;

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::get(&format!("/api/store/agents/{}", agent.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let detail: StoreAgentDetail = response.json();
    assert_eq!(detail.agent.title, "Detail Test Coach");
    assert_eq!(detail.publish_status, PublishStatus::Published);
    assert_ne!(detail.system_prompt, "");
}

#[tokio::test]
async fn test_get_agent_detail_not_found() {
    let (router, auth_token) = setup_test_environment().await;

    let fake_id = Uuid::new_v4();
    let response = AxumTestRequest::get(&format!("/api/store/agents/{fake_id}"))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_agent_detail_invalid_id() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/store/agents/invalid-uuid")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// Search Tests
// ============================================================================

#[tokio::test]
async fn test_search_agents() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Marathon Training Expert",
        AgentCategory::Training,
    )
    .await;
    create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Nutrition Advisor",
        AgentCategory::Nutrition,
    )
    .await;

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::get("/api/store/search?q=marathon")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: SearchAgentsResponse = response.json();
    assert_eq!(result.query, "marathon");
    assert_eq!(result.agents.len(), 1);
    assert_eq!(result.agents[0].title, "Marathon Training Expert");
}

#[tokio::test]
async fn test_search_agents_empty_query() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/store/search?q=")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_search_agents_no_results() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/store/search?q=nonexistent")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: SearchAgentsResponse = response.json();
    assert!(result.agents.is_empty());
}

// ============================================================================
// Categories Tests
// ============================================================================

// ============================================================================
// Install Agent Tests
// ============================================================================

/// carnet#1096: a catalogue agent joins the athlete's team as itself — the
/// response is the origin, with the install counted, and no copy is written.
#[tokio::test]
async fn test_install_agent_puts_the_catalogue_agent_on_the_team() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let agent = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Installable Coach",
        AgentCategory::Training,
    )
    .await;

    // Create a second user who will install the agent
    let (installer_id, user2) =
        create_test_user_with_email(&resources.agent.database, "user2@example.com")
            .await
            .unwrap();
    let token = generate_test_token(&resources, &user2).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));
    let installer_tenant = tenant_of(&resources, installer_id).await;
    let owned_before = resources
        .common
        .repos
        .agents
        .count(installer_id, installer_tenant)
        .await
        .unwrap();
    let (events, _guard) = capture_notify();

    let response = AxumTestRequest::post(&format!("/api/store/agents/{}/install", agent.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let result: InstallAgentResponse = response.json();
    assert!(result.message.contains("Successfully installed"));
    assert_eq!(result.agent.title, "Installable Coach");
    assert_eq!(
        result.agent.id, agent.id,
        "the catalogue agent itself, never a copy"
    );
    assert_eq!(result.agent.install_count, 1, "this install is counted");
    assert_eq!(
        resources
            .common
            .repos
            .agents
            .count(installer_id, installer_tenant)
            .await
            .unwrap(),
        owned_before,
        "no agents row is written for the installer"
    );

    // `agent.installed` fires once, from the install service this route
    // shares with the `install_agent_from_store` tool and `/discover install`.
    let installed = only(&events, "agent.installed");
    assert_eq!(installed.field("agent_slug"), agent.id.to_string());
    assert_eq!(installed.field("user_id"), installer_id.to_string());
}

#[tokio::test]
async fn test_install_agent_already_installed() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let agent = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Already Installed",
        AgentCategory::Training,
    )
    .await;

    // Create a second user
    let (_user2_id, user2) =
        create_test_user_with_email(&resources.agent.database, "user2@example.com")
            .await
            .unwrap();
    let token = generate_test_token(&resources, &user2).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let (events, _guard) = capture_notify();

    // Install once
    AxumTestRequest::post(&format!("/api/store/agents/{}/install", agent.id))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    // Try to install again
    let response = AxumTestRequest::post(&format!("/api/store/agents/{}/install", agent.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        named(&events, "agent.installed").len(),
        1,
        "a refused second install is not counted"
    );
}

#[tokio::test]
async fn test_install_agent_not_found() {
    let (router, auth_token) = setup_test_environment().await;

    let fake_id = Uuid::new_v4();
    let response = AxumTestRequest::post(&format!("/api/store/agents/{fake_id}/install"))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_install_increments_install_count() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let agent = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Count Test",
        AgentCategory::Training,
    )
    .await;
    // Freshly published agents start with install_count 0 in their StoreListing
    let original_count = 0;

    // Create a second user to install
    let (_user2_id, user2) =
        create_test_user_with_email(&resources.agent.database, "user2@example.com")
            .await
            .unwrap();
    let token = generate_test_token(&resources, &user2).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    AxumTestRequest::post(&format!("/api/store/agents/{}/install", agent.id))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    // Verify install count increased
    let response = AxumTestRequest::get(&format!("/api/store/agents/{}", agent.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    let detail: StoreAgentDetail = response.json();
    assert_eq!(detail.agent.install_count, original_count + 1);
}

// ============================================================================
// Uninstall Agent Tests
// ============================================================================

/// Uninstalling a catalogue agent ends the membership by the origin's id; the
/// row stays with its usage, so installing again resumes the same counters.
#[tokio::test]
async fn test_uninstall_agent_ends_the_membership_and_keeps_its_counters() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let source_coach = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Uninstall Test",
        AgentCategory::Training,
    )
    .await;

    // Create a second user to install then uninstall
    let (athlete_id, athlete) =
        create_test_user_with_email(&resources.agent.database, "athlete@example.com")
            .await
            .unwrap();
    let athlete_tenant = tenant_of(&resources, athlete_id).await;
    let token = generate_test_token(&resources, &athlete).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));
    let repos = &resources.common.repos;
    let source_id = source_coach.id.to_string();

    // Install first, then use it three times.
    let install_response = AxumTestRequest::post(&format!("/api/store/agents/{source_id}/install"))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;
    let installed: InstallAgentResponse = install_response.json();
    assert_eq!(installed.agent.id, source_coach.id);
    for _ in 0..3 {
        assert!(repos
            .agents
            .record_usage(&source_id, athlete_id, athlete_tenant)
            .await
            .unwrap());
    }

    // Uninstall by the origin's id: the membership ends.
    let response = AxumTestRequest::delete(&format!("/api/store/agents/{source_id}/install"))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: UninstallAgentResponse = response.json();
    assert!(result.message.contains("uninstalled"));
    assert_eq!(result.source_agent_id, source_id);

    assert!(repos
        .store_listings
        .get_installed_agents(athlete_id, athlete_tenant)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .agents
        .list_assignments(&source_id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(listing_install_count(&resources, &source_id).await, 0);
    let state = repos
        .agents
        .get_user_preferences(&source_id, athlete_id)
        .await
        .unwrap();
    assert!(!state.is_member, "uninstalling ends the membership");
    let (use_count, last_used_at) = (state.use_count, state.last_used_at);
    assert_eq!(use_count, 3, "leaving the team keeps the usage counters");
    assert!(last_used_at.is_some());

    // A second uninstall finds no membership.
    let again = AxumTestRequest::delete(&format!("/api/store/agents/{source_id}/install"))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;
    assert_eq!(again.status_code(), StatusCode::NOT_FOUND);

    // Installing again promotes the same row: the counters are still there.
    let reinstall = AxumTestRequest::post(&format!("/api/store/agents/{source_id}/install"))
        .header("authorization", &auth_token)
        .send(router)
        .await;
    assert_eq!(reinstall.status_code(), StatusCode::CREATED);
    let state = repos
        .agents
        .get_user_preferences(&source_id, athlete_id)
        .await
        .unwrap();
    assert!(state.is_member, "the re-install is a membership again");
    assert_eq!(state.use_count, 3);
    assert_eq!(listing_install_count(&resources, &source_id).await, 1);
}

#[tokio::test]
async fn test_uninstall_agent_not_from_store() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    // The user's own agent, created in the editor: not a Store install.
    let request: CreateAgentRequest = serde_json::from_value(serde_json::json!({
        "title": "Not From Store",
        "system_prompt": "Prompt",
        "category": "training",
        "tags": [],
    }))
    .unwrap();
    let agent = resources
        .common
        .repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();

    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::delete(&format!("/api/store/agents/{}/install", agent.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "an agent the user wrote is not uninstallable from the Store"
    );
    assert!(
        resources
            .common
            .repos
            .agents
            .get_by_id(&agent.id.to_string(), user_id, tenant_id)
            .await
            .unwrap()
            .is_some(),
        "the refused uninstall deletes nothing"
    );
}

#[tokio::test]
async fn test_uninstall_agent_not_found() {
    let (router, auth_token) = setup_test_environment().await;

    let fake_id = Uuid::new_v4();
    let response = AxumTestRequest::delete(&format!("/api/store/agents/{fake_id}/install"))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

/// Another tenant's athlete-published agent keeps the reviewed-copy install:
/// the copy carries the membership, is the one the installations list shows,
/// and is uninstalled by its own id.
#[tokio::test]
async fn test_install_of_a_user_published_agent_installs_a_reviewed_copy() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, _author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin =
        create_user_published_agent(&resources, author_id, author_tenant, "Hill Coach").await;
    assert!(!origin.is_system);
    let origin_id = origin.id.to_string();

    let (copier_id, copier) =
        create_test_user_with_email(&resources.agent.database, "copy-copier@example.com")
            .await
            .unwrap();
    let copier_tenant = tenant_of(&resources, copier_id).await;
    let auth = format!("Bearer {}", generate_test_token(&resources, &copier).await);
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);
    let installed: InstallAgentResponse = response.json();
    let copy_id = installed.agent.id;
    assert_ne!(copy_id, origin.id, "another tenant's agent is copied");
    let copy = resources
        .common
        .repos
        .agents
        .get_by_id(&copy_id.to_string(), copier_id, copier_tenant)
        .await
        .unwrap()
        .expect("the copy is the copier's");
    assert_eq!(copy.forked_from, Some(origin.id));
    assert_eq!(listing_install_count(&resources, &origin_id).await, 1);

    let again = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(
        again.status_code(),
        StatusCode::BAD_REQUEST,
        "holding a copy is already installed"
    );

    let listed: InstallationsResponse = AxumTestRequest::get("/api/store/installations")
        .header("authorization", &auth)
        .send(router.clone())
        .await
        .json();
    assert_eq!(
        listed.agents.iter().map(|a| a.id).collect::<Vec<_>>(),
        vec![copy_id]
    );

    // The copy is uninstalled by its own id; the origin's id finds no
    // membership to end.
    let by_origin = AxumTestRequest::delete(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(by_origin.status_code(), StatusCode::NOT_FOUND);
    let response = AxumTestRequest::delete(&format!("/api/store/agents/{copy_id}/install"))
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let result: UninstallAgentResponse = response.json();
    assert_eq!(result.source_agent_id, origin_id);
    assert!(resources
        .common
        .repos
        .agents
        .get_by_id(&copy_id.to_string(), copier_id, copier_tenant)
        .await
        .unwrap()
        .is_none());
    assert_eq!(listing_install_count(&resources, &origin_id).await, 0);
}

/// Uninstalling an athlete's edited copy deletes it, and the conversation that
/// ran on it moves to the catalogue agent instead of blocking the delete.
#[tokio::test]
async fn test_uninstall_of_a_forked_copy_hands_its_conversation_to_the_origin() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, _author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin = create_published_agent(
        &resources,
        author_id,
        author_tenant,
        "Forkable Coach",
        AgentCategory::Training,
    )
    .await;
    let origin_id = origin.id.to_string();

    let (athlete_id, athlete) =
        create_test_user_with_email(&resources.agent.database, "forker@example.com")
            .await
            .unwrap();
    let athlete_tenant = tenant_of(&resources, athlete_id).await;
    let auth = format!("Bearer {}", generate_test_token(&resources, &athlete).await);
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));
    let repos = &resources.common.repos;

    let response = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);
    let copy = repos
        .agents
        .fork_agent(&origin_id, athlete_id, athlete_tenant)
        .await
        .unwrap();
    let copy_id = copy.id.to_string();
    let conversation = repos
        .chat
        .create_conversation(
            &athlete_id.to_string(),
            athlete_tenant,
            "On the copy",
            "test-model",
            Some(&copy_id),
            None,
        )
        .await
        .unwrap()
        .id;
    assert_eq!(
        listing_install_count(&resources, &origin_id).await,
        1,
        "a fork moves the membership, it is not a second install"
    );

    let response = AxumTestRequest::delete(&format!("/api/store/agents/{copy_id}/install"))
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let result: UninstallAgentResponse = response.json();
    assert_eq!(result.source_agent_id, origin_id);

    assert!(repos
        .agents
        .get_by_id(&copy_id, athlete_id, athlete_tenant)
        .await
        .unwrap()
        .is_none());
    let moved = repos
        .chat
        .get_conversation(&conversation, &athlete_id.to_string(), athlete_tenant)
        .await
        .unwrap()
        .expect("the conversation survives the uninstall");
    assert_eq!(moved.agent_id.as_deref(), Some(origin_id.as_str()));
    assert_eq!(listing_install_count(&resources, &origin_id).await, 0);
}

/// Catalogue agents are owned by the operator who wrote them. An operator who
/// also coaches with one installs it like anyone else, and can take it off
/// their team again.
#[tokio::test]
async fn test_the_author_of_a_catalogue_agent_can_take_it_off_their_team() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin = create_published_agent(
        &resources,
        author_id,
        author_tenant,
        "Operator Coach",
        AgentCategory::Training,
    )
    .await;
    let origin_id = origin.id.to_string();
    let auth = format!("Bearer {}", generate_test_token(&resources, &author).await);
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);

    let response = AxumTestRequest::delete(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    assert!(resources
        .common
        .repos
        .store_listings
        .get_installed_agents(author_id, author_tenant)
        .await
        .unwrap()
        .is_empty());
    assert!(
        resources
            .common
            .repos
            .agents
            .get_system_agent(&origin_id, author_tenant)
            .await
            .unwrap()
            .is_some(),
        "leaving the team never deletes the catalogue agent"
    );
}

/// The install counter counts members plus live copies. A fork by an athlete
/// who was never a member adds a copy, so uninstalling that copy later must not
/// take an install away from someone else.
#[tokio::test]
async fn test_a_fork_without_a_membership_never_skews_the_install_count() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, _author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin = create_published_agent(
        &resources,
        author_id,
        author_tenant,
        "Counted Coach",
        AgentCategory::Training,
    )
    .await;
    let origin_id = origin.id.to_string();
    let repos = &resources.common.repos;
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let (member_id, member) =
        create_test_user_with_email(&resources.agent.database, "member@example.com")
            .await
            .unwrap();
    let member_auth = format!("Bearer {}", generate_test_token(&resources, &member).await);
    let response = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &member_auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);
    assert!(repos
        .agents
        .list_assignments(&origin_id)
        .await
        .unwrap()
        .iter()
        .any(|a| a.user_id == member_id.to_string()));
    assert_eq!(listing_install_count(&resources, &origin_id).await, 1);

    // Someone who only used the agent edits it: a fork, never installed.
    let (forker_id, forker) =
        create_test_user_with_email(&resources.agent.database, "forker@example.com")
            .await
            .unwrap();
    let forker_tenant = tenant_of(&resources, forker_id).await;
    assert!(repos
        .agents
        .record_usage(&origin_id, forker_id, forker_tenant)
        .await
        .unwrap());
    let copy = repos
        .agents
        .fork_agent(&origin_id, forker_id, forker_tenant)
        .await
        .unwrap();
    assert_eq!(
        listing_install_count(&resources, &origin_id).await,
        2,
        "the new copy is counted like an install"
    );

    let forker_auth = format!("Bearer {}", generate_test_token(&resources, &forker).await);
    let response = AxumTestRequest::delete(&format!("/api/store/agents/{}/install", copy.id))
        .header("authorization", &forker_auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(
        listing_install_count(&resources, &origin_id).await,
        1,
        "the member's install is still counted"
    );
}

/// Favouriting, activating and an admin assignment make a membership without
/// a Store install, and a Store Remove (or an admin unassign) can end it. The
/// counter follows every membership change, so ending one never takes away an
/// install nobody counted.
#[tokio::test]
async fn test_every_membership_change_moves_the_install_count() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, _author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin = create_published_agent(
        &resources,
        author_id,
        author_tenant,
        "Membership Coach",
        AgentCategory::Training,
    )
    .await;
    let origin_id = origin.id.to_string();
    let repos = &resources.common.repos;
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));
    assert_eq!(listing_install_count(&resources, &origin_id).await, 0);

    // A favourite joins the team and counts; Remove in the Store uncounts it.
    let (fan_id, fan) = create_test_user_with_email(&resources.agent.database, "fan@example.com")
        .await
        .unwrap();
    let fan_tenant = tenant_of(&resources, fan_id).await;
    assert_eq!(
        repos
            .agents
            .toggle_favorite(&origin_id, fan_id, fan_tenant)
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(listing_install_count(&resources, &origin_id).await, 1);
    let fan_auth = format!("Bearer {}", generate_test_token(&resources, &fan).await);
    let again = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &fan_auth)
        .send(router.clone())
        .await;
    assert_eq!(
        again.status_code(),
        StatusCode::BAD_REQUEST,
        "a favourite is already on the team"
    );
    let removed = AxumTestRequest::delete(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &fan_auth)
        .send(router.clone())
        .await;
    assert_eq!(removed.status_code(), StatusCode::OK);
    assert_eq!(
        listing_install_count(&resources, &origin_id).await,
        0,
        "the favourite's install was counted, so its removal leaves the real count"
    );

    // Activating joins the team once; a second activation changes nothing.
    let (picker_id, _picker) =
        create_test_user_with_email(&resources.agent.database, "picker@example.com")
            .await
            .unwrap();
    let picker_tenant = tenant_of(&resources, picker_id).await;
    for _ in 0..2 {
        assert!(repos
            .agents
            .activate_agent(&origin_id, picker_id, picker_tenant)
            .await
            .unwrap()
            .is_some());
    }
    assert_eq!(listing_install_count(&resources, &origin_id).await, 1);

    // Usage never counts.
    let (user_id, _user) =
        create_test_user_with_email(&resources.agent.database, "user@example.com")
            .await
            .unwrap();
    let user_tenant = tenant_of(&resources, user_id).await;
    assert!(repos
        .agents
        .record_usage(&origin_id, user_id, user_tenant)
        .await
        .unwrap());
    assert_eq!(listing_install_count(&resources, &origin_id).await, 1);

    // An admin assignment of that usage row counts; unassigning uncounts it.
    assert!(repos
        .agents
        .assign_agent(&origin_id, user_id, author_id)
        .await
        .unwrap());
    assert_eq!(listing_install_count(&resources, &origin_id).await, 2);
    assert!(repos
        .agents
        .unassign_agent(&origin_id, user_id)
        .await
        .unwrap());
    assert!(!repos
        .agents
        .unassign_agent(&origin_id, user_id)
        .await
        .unwrap());
    assert_eq!(
        listing_install_count(&resources, &origin_id).await,
        1,
        "only the picker is still on the team"
    );
}

/// The author of an athlete-published agent sees their own listing as their
/// own: it is on their team as their creation, not as a Store install, so the
/// clients offer neither Install nor Remove. Anyone else sees a listing.
#[tokio::test]
async fn test_the_author_sees_their_own_listing_as_their_own() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin =
        create_user_published_agent(&resources, author_id, author_tenant, "Own Coach").await;
    let (_other_id, other) =
        create_test_user_with_email(&resources.agent.database, "viewer@example.com")
            .await
            .unwrap();
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    for (user, expected) in [(&author, true), (&other, false)] {
        let auth = format!("Bearer {}", generate_test_token(&resources, user).await);
        let response = AxumTestRequest::get(&format!("/api/store/agents/{}", origin.id))
            .header("authorization", &auth)
            .send(router.clone())
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let detail: StoreAgentDetail = response.json();
        assert_eq!(detail.is_own, expected, "{}", user.email);
    }

    // A catalogue agent is never "own", even to the operator who wrote it:
    // they install and remove it like anyone.
    let catalogue = create_published_agent(
        &resources,
        author_id,
        author_tenant,
        "Catalogue Coach",
        AgentCategory::Training,
    )
    .await;
    let auth = format!("Bearer {}", generate_test_token(&resources, &author).await);
    let detail: StoreAgentDetail =
        AxumTestRequest::get(&format!("/api/store/agents/{}", catalogue.id))
            .header("authorization", &auth)
            .send(router)
            .await
            .json();
    assert!(!detail.is_own);
}

/// A reviewed copy of another tenant's agent that still runs a coaching group
/// cannot be uninstalled: the group's tenant cannot run the origin, and a
/// group cannot go without an agent. The refusal changes nothing.
#[tokio::test]
async fn test_a_copy_running_a_group_its_tenant_cannot_move_is_not_uninstalled() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, _author) = create_test_user(&resources.agent.database).await.unwrap();
    let author_tenant = tenant_of(&resources, author_id).await;
    let origin =
        create_user_published_agent(&resources, author_id, author_tenant, "Group Coach").await;
    let origin_id = origin.id.to_string();

    let (copier_id, copier) =
        create_test_user_with_email(&resources.agent.database, "group-copier@example.com")
            .await
            .unwrap();
    let copier_tenant = tenant_of(&resources, copier_id).await;
    let auth = format!("Bearer {}", generate_test_token(&resources, &copier).await);
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));
    let response = AxumTestRequest::post(&format!("/api/store/agents/{origin_id}/install"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);
    let copy_id = response.json::<InstallAgentResponse>().agent.id.to_string();

    let group_id = resources
        .group_service()
        .create_group(
            &CreateGroupRequest {
                name: "Copy Crew".to_owned(),
                description: None,
                agent_id: copy_id.clone(),
                max_members: None,
                coach_user_id: None,
            },
            copier_id,
            copier_tenant,
            &EnterpriseTierStrategy,
        )
        .await
        .unwrap()
        .id;

    let response = AxumTestRequest::delete(&format!("/api/store/agents/{copy_id}/install"))
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert!(
        resources
            .common
            .repos
            .agents
            .get_by_id(&copy_id, copier_id, copier_tenant)
            .await
            .unwrap()
            .is_some(),
        "the refused uninstall deletes nothing"
    );
    let group = resources
        .common
        .repos
        .groups
        .get_group(&group_id.to_string(), copier_tenant)
        .await
        .unwrap()
        .expect("the group is still there");
    assert_eq!(group.agent_id, copy_id, "the group still runs on the copy");
    assert_eq!(listing_install_count(&resources, &origin_id).await, 1);
}

// ============================================================================
// List Installations Tests
// ============================================================================

#[tokio::test]
async fn test_list_installations_empty() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/store/installations")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: InstallationsResponse = response.json();
    assert!(result.agents.is_empty());
}

#[tokio::test]
async fn test_list_installations() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database).await.unwrap();

    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants
        .first()
        .map_or_else(|| TenantId::from_uuid(user_id), |t| t.id);

    let coach1 = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Install 1",
        AgentCategory::Training,
    )
    .await;
    let coach2 = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Install 2",
        AgentCategory::Nutrition,
    )
    .await;

    // Create a second user to install
    let (_user2_id, user2) =
        create_test_user_with_email(&resources.agent.database, "user2@example.com")
            .await
            .unwrap();
    let token = generate_test_token(&resources, &user2).await;
    let auth_token = format!("Bearer {token}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Install both agents
    AxumTestRequest::post(&format!("/api/store/agents/{}/install", coach1.id))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;
    AxumTestRequest::post(&format!("/api/store/agents/{}/install", coach2.id))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    // List installations
    let response = AxumTestRequest::get("/api/store/installations")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: InstallationsResponse = response.json();
    let mut ids: Vec<Uuid> = result.agents.iter().map(|a| a.id).collect();
    ids.sort();
    let mut expected = vec![coach1.id, coach2.id];
    expected.sort();
    assert_eq!(ids, expected, "the catalogue agents themselves, no copies");
}

// ============================================================================
// Multi-Tenant Isolation Tests
// ============================================================================

#[tokio::test]
async fn test_published_agents_visible_cross_tenant() {
    let resources = create_test_server_resources().await.unwrap();

    // User 1 in tenant 1 creates a published agent
    let (user1_id, _user1) =
        create_test_user_with_email(&resources.agent.database, "user1@example.com")
            .await
            .unwrap();
    let tenants1 = resources
        .common
        .repos
        .tenants
        .list_for_user(user1_id)
        .await
        .unwrap();
    let tenant1_id = tenants1
        .first()
        .map_or_else(|| TenantId::from_uuid(user1_id), |t| t.id);

    let agent = create_published_agent(
        &resources,
        user1_id,
        tenant1_id,
        "Cross Tenant Coach",
        AgentCategory::Training,
    )
    .await;

    // User 2 in tenant 2 should see the published agent
    let (_user2_id, user2) =
        create_test_user_with_email(&resources.agent.database, "user2@example.com")
            .await
            .unwrap();
    let token2 = generate_test_token(&resources, &user2).await;
    let auth_token2 = format!("Bearer {token2}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    let response = AxumTestRequest::get("/api/store/agents")
        .header("authorization", &auth_token2)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let result: BrowseAgentsResponse = response.json();
    assert_eq!(result.agents.len(), 1);
    assert_eq!(result.agents[0].id, agent.id);
}

#[tokio::test]
async fn test_installations_isolated_per_user() {
    let resources = create_test_server_resources().await.unwrap();

    // User 1 creates a published agent
    let (user1_id, _user1) =
        create_test_user_with_email(&resources.agent.database, "user1@example.com")
            .await
            .unwrap();
    let tenants1 = resources
        .common
        .repos
        .tenants
        .list_for_user(user1_id)
        .await
        .unwrap();
    let tenant1_id = tenants1
        .first()
        .map_or_else(|| TenantId::from_uuid(user1_id), |t| t.id);

    let agent = create_published_agent(
        &resources,
        user1_id,
        tenant1_id,
        "Install Test",
        AgentCategory::Training,
    )
    .await;

    // User 2 installs the agent
    let (_user2_id, user2) =
        create_test_user_with_email(&resources.agent.database, "user2@example.com")
            .await
            .unwrap();
    let token2 = generate_test_token(&resources, &user2).await;
    let auth_token2 = format!("Bearer {token2}");
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));

    AxumTestRequest::post(&format!("/api/store/agents/{}/install", agent.id))
        .header("authorization", &auth_token2)
        .send(router.clone())
        .await;

    // User 3 should have no installations
    let (_user3_id, user3) =
        create_test_user_with_email(&resources.agent.database, "user3@example.com")
            .await
            .unwrap();
    let token3 = generate_test_token(&resources, &user3).await;
    let auth_token3 = format!("Bearer {token3}");

    let response = AxumTestRequest::get("/api/store/installations")
        .header("authorization", &auth_token3)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let result: InstallationsResponse = response.json();
    assert!(result.agents.is_empty());
}

// ============================================================================
// Store Health Check Test
// ============================================================================

// ============================================================================
// Coach-facing agents
// ============================================================================

/// An athlete never finds an agent written for a coach — browsing or
/// searching — while a coach finds it beside the rest.
#[tokio::test]
async fn test_store_shows_coach_facing_agents_to_coaches_only() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;

    let athlete_agent = create_published_agent(
        &resources,
        user_id,
        tenant_id,
        "Marathon Agent",
        AgentCategory::Training,
    )
    .await;
    let roster_agent = create_published_agent_tagged(
        &resources,
        user_id,
        tenant_id,
        "Roster Agent",
        AgentCategory::Custom,
        vec![Agent::COACH_TOOL_TAG.to_owned(), "roster".to_owned()],
    )
    .await;

    let auth_token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(&resources));
    let browse_ids = |router: axum::Router, auth: String| async move {
        let result: BrowseAgentsResponse = AxumTestRequest::get("/api/store/agents")
            .header("authorization", &auth)
            .send(router)
            .await
            .json();
        result.agents.into_iter().map(|a| a.id).collect::<Vec<_>>()
    };
    let search_ids = |router: axum::Router, auth: String| async move {
        let result: SearchAgentsResponse = AxumTestRequest::get("/api/store/search?q=agent")
            .header("authorization", &auth)
            .send(router)
            .await
            .json();
        result.agents.into_iter().map(|a| a.id).collect::<Vec<_>>()
    };

    assert_eq!(
        browse_ids(router.clone(), auth_token.clone()).await,
        vec![athlete_agent.id],
        "an athlete browses the athlete-facing catalogue only"
    );
    assert_eq!(
        search_ids(router.clone(), auth_token.clone()).await,
        vec![athlete_agent.id],
        "an athlete's search never surfaces a coach-facing agent"
    );
    let install = |router: axum::Router, auth: String| async move {
        AxumTestRequest::post(&format!("/api/store/agents/{}/install", roster_agent.id))
            .header("authorization", &auth)
            .send(router)
            .await
            .status_code()
    };
    assert_eq!(
        install(router.clone(), auth_token.clone()).await,
        StatusCode::NOT_FOUND,
        "an athlete cannot install a coach-facing agent by its id either"
    );

    resources
        .common
        .repos
        .users
        .set_coaches_others(user_id, true)
        .await
        .unwrap();
    let mut coach_browse = browse_ids(router.clone(), auth_token.clone()).await;
    coach_browse.sort();
    let mut expected = vec![athlete_agent.id, roster_agent.id];
    expected.sort();
    assert_eq!(coach_browse, expected, "a coach browses both");
    let mut coach_search = search_ids(router.clone(), auth_token.clone()).await;
    coach_search.sort();
    assert_eq!(coach_search, expected, "a coach finds both");
    assert_eq!(
        install(router, auth_token).await,
        StatusCode::CREATED,
        "a coach installs it"
    );
}
