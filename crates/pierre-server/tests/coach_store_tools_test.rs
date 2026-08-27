// ABOUTME: The Coach Store is reachable from a chat turn — browse, search and install as MCP tools
// ABOUTME: Content-asserting: real published coaches through the executor, then a real installed copy

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#62: the coach store was invisible to every chat surface.
//!
//! `CHAT_CALLABLE_CATEGORIES` named no store category, so the LLM was never
//! shown a tool that could browse or install a coach — on web, on mobile, and
//! in messaging alike. An athlete asking "what nutrition coaches do you have?"
//! got a truthful refusal from a product whose whole marketplace sat one table
//! away.
//!
//! These assert the fix by value, not by the existence of a registration:
//! the three tools appear in the chat-callable schema set, browse returns the
//! published coach by title, search finds it by a word from that title, and
//! install leaves a real second row the athlete owns.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::TenantId;
use pierre_database::database::coaches::{
    CoachCategory, CoachVisibility, CoachesManager, CreateSystemCoachRequest,
};
use pierre_database::database::StoreListingsManager;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;
mod helpers;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    let email = format!("store_tools_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("a fresh user owns a tenant");
    Ok((user_id, tenant.id))
}

/// Publish a coach into the Store, exactly as `store_routes_test` does, so the
/// tools read the same rows the REST surface does.
async fn publish_coach(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    category: CoachCategory,
) -> Uuid {
    let pool = executor
        .resources
        .database()
        .sqlite_pool()
        .expect("tests run on sqlite")
        .clone();
    let coaches = CoachesManager::new(pool.clone());
    let listings = StoreListingsManager::new(pool);

    let coach = coaches
        .create_system_coach(
            user_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: format!("You are a {title} coach."),
                category,
                tags: vec!["test".to_owned()],
                visibility: CoachVisibility::Tenant,
                sample_prompts: vec!["Sample prompt".to_owned()],
            },
        )
        .await
        .unwrap();

    listings
        .submit_for_review(&coach.id.to_string(), user_id, tenant_id)
        .await
        .unwrap();
    listings
        .approve_coach(&coach.id.to_string(), tenant_id, user_id)
        .await
        .unwrap();
    coach.id
}

fn make_request(tool: &str, params: Value, user_id: Uuid, tenant_id: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

async fn run(
    executor: &UniversalToolExecutor,
    tool: &str,
    params: Value,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Value {
    let response = executor
        .execute_tool(make_request(tool, params, user_id, tenant_id))
        .await
        .unwrap_or_else(|e| panic!("{tool} should have executed: {e}"));
    assert!(
        response.success,
        "{tool} should have succeeded: {:?}",
        response.error
    );
    response.result.expect("tool returns a payload")
}

/// The registration half: all three store tools are on the surface the LLM is
/// offered during a chat turn. This is the exact assertion that would have
/// failed before the `store` category joined `CHAT_CALLABLE_CATEGORIES`.
#[test]
fn store_tools_are_chat_callable() {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);

    let chat_callable: Vec<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|s| s.name)
        .collect();

    for tool in [
        "browse_coach_store",
        "search_coach_store",
        "install_coach_from_store",
    ] {
        assert!(
            chat_callable.iter().any(|n| n == tool),
            "{tool} must be chat-callable; chat-callable set was {chat_callable:?}"
        );
    }

    // The store category carries exactly these three. Uninstall is deliberately
    // absent: removing a coach the athlete has history with is a UI gesture,
    // not an inference from a sentence.
    let mut in_category = registry.tools_in_category("store");
    in_category.sort_unstable();
    assert_eq!(
        in_category,
        vec![
            "browse_coach_store",
            "install_coach_from_store",
            "search_coach_store"
        ],
        "the store category must hold exactly the three browse/search/install tools"
    );
}

#[tokio::test]
async fn browse_coach_store_returns_published_coaches() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    publish_coach(
        &executor,
        user_id,
        tenant_id,
        "Ultra Trail Fuelling",
        CoachCategory::Nutrition,
    )
    .await;

    let payload = run(
        &executor,
        "browse_coach_store",
        json!({}),
        user_id,
        tenant_id,
    )
    .await;

    assert_eq!(
        payload["count"], 1,
        "one published coach was created, so browse returns exactly one: {payload}"
    );
    assert_eq!(payload["coaches"][0]["title"], "Ultra Trail Fuelling");
    assert_eq!(payload["coaches"][0]["category"], "nutrition");
    assert_eq!(
        payload["has_more"], false,
        "a single-coach store has no further page"
    );
    Ok(())
}

#[tokio::test]
async fn search_coach_store_matches_published_titles() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    publish_coach(
        &executor,
        user_id,
        tenant_id,
        "Ultra Trail Fuelling",
        CoachCategory::Nutrition,
    )
    .await;
    publish_coach(
        &executor,
        user_id,
        tenant_id,
        "Track Speed Blocks",
        CoachCategory::Training,
    )
    .await;

    let hit = run(
        &executor,
        "search_coach_store",
        json!({ "query": "Fuelling" }),
        user_id,
        tenant_id,
    )
    .await;
    assert_eq!(
        hit["count"], 1,
        "only one of the two published coaches matches 'Fuelling': {hit}"
    );
    assert_eq!(hit["coaches"][0]["title"], "Ultra Trail Fuelling");

    let miss = run(
        &executor,
        "search_coach_store",
        json!({ "query": "kayaking" }),
        user_id,
        tenant_id,
    )
    .await;
    assert_eq!(miss["count"], 0, "no published coach matches 'kayaking'");
    Ok(())
}

#[tokio::test]
async fn install_coach_from_store_creates_the_athletes_own_copy() -> Result<()> {
    let executor = create_executor().await?;
    let (author_id, tenant_id) = create_test_user(&executor).await?;
    let published = publish_coach(
        &executor,
        author_id,
        tenant_id,
        "Ultra Trail Fuelling",
        CoachCategory::Nutrition,
    )
    .await;

    let installer_email = format!("store_installer_{}@example.com", Uuid::new_v4());
    let (installer_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &installer_email)
            .await?;

    let (events, _guard) = helpers::notify_capture::capture_notify();
    let payload = run(
        &executor,
        "install_coach_from_store",
        json!({ "coach_id": published.to_string() }),
        installer_id,
        tenant_id,
    )
    .await;

    assert_eq!(payload["installed"], true);
    assert_eq!(payload["coach"]["title"], "Ultra Trail Fuelling");

    // `coach.installed` fires once, from the install service this tool
    // shares with the REST route and `/discover install`.
    let installed = helpers::notify_capture::only(&events, "coach.installed");
    assert_eq!(installed.field("coach_slug"), published.to_string());
    assert_eq!(installed.field("user_id"), installer_id.to_string());

    let copy_coach_id = payload["coach"]["id"]
        .as_str()
        .expect("the installed copy carries its own id");
    assert_ne!(
        copy_coach_id,
        published.to_string(),
        "install must create the athlete's own copy, not hand back the listing row"
    );

    // And the copy is really in the installer's library.
    let pool = executor
        .resources
        .database()
        .sqlite_pool()
        .expect("tests run on sqlite")
        .clone();
    let installed = StoreListingsManager::new(pool)
        .get_installed_coaches(installer_id, tenant_id)
        .await?;
    assert_eq!(
        installed.len(),
        1,
        "exactly the one installed coach is in the library"
    );
    assert_eq!(installed[0].title, "Ultra Trail Fuelling");
    Ok(())
}
