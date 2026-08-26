// ABOUTME: Integration tests for the coach catalogue handle on SQLite
// ABOUTME: Approval assigns a unique @handle, installs carry it, and only installed coaches resolve
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use axum::http::StatusCode;
use common::{
    create_test_server_resources, create_test_user, create_test_user_with_plan, generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::coaches::{
    CoachCategory, CoachHandle, CoachVisibility, CreateSystemCoachRequest,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_coaches::build_coaches_router;
use pierre_routes_coaches::coaches::CoachResponse;
use std::sync::Arc;
use uuid::Uuid;

/// Create a system coach and take it through review to a published listing.
async fn publish_coach(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    tenant_id: TenantId,
    title: &str,
) -> Uuid {
    let coach = repos
        .coaches
        .create_system_coach(
            author_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: format!("You are the {title}."),
                category: CoachCategory::Training,
                tags: vec!["test".to_owned()],
                visibility: CoachVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        coach.handle, None,
        "a coach owns no handle before it enters the catalogue"
    );

    let id = coach.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author_id, tenant_id)
        .await
        .unwrap();
    repos
        .store_listings
        .approve_coach(&id, tenant_id, Some(author_id))
        .await
        .unwrap();
    coach.id
}

#[test]
fn handle_derives_from_title_and_numbers_collisions() {
    let base = CoachHandle::derive("  Tempo & Threshold Coach! ");
    assert_eq!(base.as_str(), "tempo-threshold-coach");
    assert_eq!(base.candidate(0).as_str(), "tempo-threshold-coach");
    assert_eq!(base.candidate(1).as_str(), "tempo-threshold-coach-2");
    assert_eq!(base.candidate(9).as_str(), "tempo-threshold-coach-10");

    let long = CoachHandle::derive(&"x".repeat(60));
    assert_eq!(long.as_str().len(), CoachHandle::MAX_LEN);
    assert_eq!(long.candidate(1).as_str().len(), CoachHandle::MAX_LEN);
    assert!(long.candidate(1).as_str().ends_with("-2"));

    assert_eq!(CoachHandle::derive("!!!").as_str(), "coach");
}

#[test]
fn handle_parse_accepts_the_catalogue_alphabet_and_rejects_the_rest() {
    assert_eq!(
        CoachHandle::parse("@coach-tempo").unwrap().as_str(),
        "coach-tempo"
    );
    assert_eq!(
        CoachHandle::parse("strength_v2").unwrap().as_str(),
        "strength_v2"
    );
    for bad in [
        "",
        "-leading",
        "Upper",
        "with space",
        "acc/ent",
        &"a".repeat(41),
    ] {
        assert!(CoachHandle::parse(bad).is_err(), "{bad:?} must be rejected");
    }
}

#[tokio::test]
async fn approval_assigns_a_catalogue_unique_handle() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (author_id, _author, tenant_id) =
        create_test_user_with_plan(&resources.coach.database, "author@handle.test", "starter")
            .await
            .unwrap();

    let first = publish_coach(repos, author_id, tenant_id, "Tempo Coach").await;
    let second = publish_coach(repos, author_id, tenant_id, "Tempo Coach").await;

    let first_listed = repos
        .store_listings
        .get_published_coach(&first.to_string())
        .await
        .unwrap()
        .unwrap();
    let second_listed = repos
        .store_listings
        .get_published_coach(&second.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_listed.coach.handle.as_deref(), Some("tempo-coach"));
    assert_eq!(second_listed.coach.handle.as_deref(), Some("tempo-coach-2"));

    // The unique index is the last line of defence: a second origin row
    // claiming an owned handle is refused by the database itself.
    let pool = resources.coach.database.sqlite_pool().unwrap();
    let clash = sqlx::query(
        "INSERT INTO coaches (id, user_id, tenant_id, title, system_prompt, slug, created_at, updated_at) \
         VALUES ($1, $2, $3, 'Clash', 'prompt', 'tempo-coach', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(author_id.to_string())
    .bind(tenant_id)
    .execute(pool)
    .await;
    let err = clash.expect_err("duplicate origin handle must violate idx_coaches_handle");
    assert!(
        err.to_string().to_lowercase().contains("unique"),
        "expected a UNIQUE violation, got: {err}"
    );
}

#[tokio::test]
async fn install_carries_the_handle_and_only_installers_resolve_it() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (author_id, _author, author_tenant) =
        create_test_user_with_plan(&resources.coach.database, "author@handle.test", "starter")
            .await
            .unwrap();
    let (athlete_id, _athlete, athlete_tenant) =
        create_test_user_with_plan(&resources.coach.database, "athlete@handle.test", "starter")
            .await
            .unwrap();
    let (bystander_id, _bystander, bystander_tenant) = create_test_user_with_plan(
        &resources.coach.database,
        "bystander@handle.test",
        "starter",
    )
    .await
    .unwrap();

    let origin = publish_coach(repos, author_id, author_tenant, "Recovery Coach").await;
    let handle = CoachHandle::parse("recovery-coach").unwrap();

    // Installing across tenants copies the handle onto the athlete's row.
    let installed = repos
        .store_listings
        .install_from_store(&origin.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));
    assert_eq!(installed.forked_from, Some(origin));

    let listed = repos
        .store_listings
        .get_installed_coaches(athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].handle.as_deref(), Some("recovery-coach"));

    // The athlete resolves their own copy, not the catalogue origin.
    let resolved = repos
        .coaches
        .find_installed_by_handle(&handle, athlete_id, athlete_tenant)
        .await
        .unwrap()
        .expect("installed coach resolves by handle");
    assert_eq!(resolved.id, installed.id);
    assert_eq!(resolved.title, "Recovery Coach");

    // A user who never installed it gets nothing, even though the catalogue
    // lists the coach.
    let none = repos
        .coaches
        .find_installed_by_handle(&handle, bystander_id, bystander_tenant)
        .await
        .unwrap();
    assert!(none.is_none(), "a non-installed coach must not resolve");

    // Uninstalling withdraws the name.
    repos
        .store_listings
        .uninstall_coach(&installed.id.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();
    let gone = repos
        .coaches
        .find_installed_by_handle(&handle, athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert!(gone.is_none(), "an uninstalled coach must stop resolving");
}

#[tokio::test]
async fn fork_carries_the_handle_of_its_origin() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (author_id, _author, tenant_id) =
        create_test_user_with_plan(&resources.coach.database, "author@handle.test", "starter")
            .await
            .unwrap();
    let origin = publish_coach(repos, author_id, tenant_id, "Strength Coach").await;

    let fork = repos
        .coaches
        .fork_coach(&origin.to_string(), author_id, tenant_id)
        .await
        .unwrap();
    assert_eq!(fork.handle.as_deref(), Some("strength-coach"));

    let resolved = repos
        .coaches
        .find_installed_by_handle(
            &CoachHandle::parse("strength-coach").unwrap(),
            author_id,
            tenant_id,
        )
        .await
        .unwrap()
        .expect("the fork resolves by its origin's handle");
    assert_eq!(
        resolved.id, fork.id,
        "the user's own copy wins over the origin"
    );
}

#[tokio::test]
async fn by_handle_route_returns_the_installed_coach_and_404s_otherwise() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (author_id, _author, author_tenant) =
        create_test_user_with_plan(&resources.coach.database, "author@handle.test", "starter")
            .await
            .unwrap();
    let origin = publish_coach(repos, author_id, author_tenant, "Mobility Coach").await;

    let (athlete_id, athlete) = create_test_user(&resources.coach.database).await.unwrap();
    let athlete_tenant = repos
        .tenants
        .list_for_user(athlete_id)
        .await
        .unwrap()
        .first()
        .map(|t| t.id)
        .expect("test user owns a tenant");
    let token = generate_test_token(&resources, &athlete).await;
    let auth = format!("Bearer {token}");
    let router = build_coaches_router::<ServerContext>().with_state(Arc::clone(&resources));

    // Not installed yet: the catalogue does not leak through the route.
    let response = AxumTestRequest::get("/api/coaches/by-handle/mobility-coach")
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);

    // A malformed handle is rejected before any lookup.
    let response = AxumTestRequest::get("/api/coaches/by-handle/Not%20A%20Handle")
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let installed = repos
        .store_listings
        .install_from_store(&origin.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();

    let response = AxumTestRequest::get("/api/coaches/by-handle/mobility-coach")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body: CoachResponse = response.json();
    assert_eq!(body.id, installed.id.to_string());
    assert_eq!(body.handle.as_deref(), Some("mobility-coach"));
    assert_eq!(body.title, "Mobility Coach");
    assert_eq!(
        body.forked_from.as_deref(),
        Some(origin.to_string().as_str())
    );
}
