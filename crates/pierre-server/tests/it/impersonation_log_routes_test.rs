// ABOUTME: GET /api/admin/impersonate/sessions and /sessions/{id} — the impersonation log the admin console reads
// ABOUTME: Pins that a super admin reads each session with both parties named, and anyone else is refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::collections::HashSet;
use std::sync::Arc;

use common::{create_test_server_resources, create_test_user_with_email, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::ImpersonationRoutes;
use serde_json::Value;

async fn super_admin(resources: &Arc<ServerContext>) -> User {
    let mut user = User::new(
        "root-operator@example.com".to_owned(),
        "hash".to_owned(),
        Some("Root".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.is_admin = true;
    user.role = UserRole::SuperAdmin;
    resources.common.repos.users.create(&user).await.unwrap();
    user
}

#[tokio::test]
async fn a_super_admin_reads_the_log_and_one_session() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = super_admin(&resources).await;
    let (_, target) = create_test_user_with_email(&resources.agent.database, "athlete@example.com")
        .await
        .unwrap();
    let session = ImpersonationSession::new(operator.id, target.id, Some("Ticket 4412".to_owned()));
    resources
        .common
        .repos
        .impersonation
        .create_session(&session)
        .await
        .unwrap();
    let bearer = format!(
        "Bearer {}",
        generate_test_token(&resources, &operator).await
    );

    let listed = AxumTestRequest::get("/api/admin/impersonate/sessions")
        .header("authorization", &bearer)
        .send(ImpersonationRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(listed.status(), 200, "{}", listed.text());
    let body: Value = serde_json::from_str(&listed.text()).unwrap();
    let row = body["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["id"] == session.id.as_str()))
        .unwrap_or_else(|| panic!("the session is listed: {body}"));
    assert_eq!(row["impersonator_email"], "root-operator@example.com");
    assert_eq!(row["target_user_email"], "athlete@example.com");
    assert_eq!(row["is_active"], true);

    let one = AxumTestRequest::get(&format!("/api/admin/impersonate/sessions/{}", session.id))
        .header("authorization", &bearer)
        .send(ImpersonationRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(one.status(), 200, "{}", one.text());
    let one: Value = serde_json::from_str(&one.text()).unwrap();
    assert_eq!(one["reason"], "Ticket 4412");
}

#[tokio::test]
async fn anyone_but_a_super_admin_is_refused_the_log() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, athlete) = create_test_user_with_email(&resources.agent.database, "plain@example.com")
        .await
        .unwrap();
    let bearer = format!("Bearer {}", generate_test_token(&resources, &athlete).await);

    let refused = AxumTestRequest::get("/api/admin/impersonate/sessions")
        .header("authorization", &bearer)
        .send(ImpersonationRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(refused.status(), 403, "{}", refused.text());
}

/// The log is paged: past the first page, the cursor it returns reaches every
/// other session, so a session older than the newest hundred is never lost.
#[tokio::test]
async fn the_log_reaches_every_session_past_the_first_page() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = super_admin(&resources).await;
    let (_, target) = create_test_user_with_email(&resources.agent.database, "paged@example.com")
        .await
        .unwrap();
    let total = 101;
    for _ in 0..total {
        let session = ImpersonationSession::new(operator.id, target.id, None);
        resources
            .common
            .repos
            .impersonation
            .create_session(&session)
            .await
            .unwrap();
    }
    let bearer = format!(
        "Bearer {}",
        generate_test_token(&resources, &operator).await
    );

    let mut seen = HashSet::new();
    let mut path = "/api/admin/impersonate/sessions".to_owned();
    for _ in 0..total {
        let page = AxumTestRequest::get(&path)
            .header("authorization", &bearer)
            .send(ImpersonationRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(page.status(), 200, "{}", page.text());
        let body: Value = serde_json::from_str(&page.text()).unwrap();
        for row in body["sessions"].as_array().unwrap() {
            assert!(
                seen.insert(row["id"].as_str().unwrap().to_owned()),
                "no repeat"
            );
        }
        let (Some(before), Some(before_id)) = (
            body["next_before"].as_str(),
            body["next_before_id"].as_str(),
        ) else {
            break;
        };
        path = format!(
            "/api/admin/impersonate/sessions?before={}&before_id={before_id}",
            urlencoding::encode(before)
        );
    }
    assert_eq!(
        seen.len(),
        total,
        "bug: every session must be reachable, not only the newest hundred"
    );
}
