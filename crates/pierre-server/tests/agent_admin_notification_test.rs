// ABOUTME: Drives the admin agent routes over HTTP and reads what the athlete is told about an edit and an assignment
// ABOUTME: Each notice says what the administrator did, a repeated assignment tells nobody, and neither claims a plan change
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An administrator editing a system agent, or assigning one, used to tell
//! every athlete concerned "X updated your training plan" — no plan had
//! changed. The edit and the assignment now raise their own events, worded as
//! what happened, and an assignment that changed nothing for an athlete who
//! already had the agent raises nothing at all.

#![cfg(feature = "client-notifications")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use chrono::Utc;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::database::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::models::Notification;
use pierre_notifications::TenantId as CommereTenantId;
use pierre_routes_agents::build_agents_admin_router;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

/// The system agent the administrator assigns, then edits.
const AGENT_TITLE: &str = "Marathon Agent";
/// Its title once the administrator edits it.
const EDITED_TITLE: &str = "Marathon Agent v2";

/// A tenant membership row, filed the way the invitation flows file it.
const INSERT_MEMBER: &str =
    "INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at) \
     VALUES ($1, $2, $3, 'member', $4, $5)";

/// The admin routes, nested under `/api/admin` as the server mounts them. A
/// router is consumed by each send, so callers build one per request.
fn router(resources: &Arc<ServerContext>) -> axum::Router {
    axum::Router::new()
        .nest("/api/admin", build_agents_admin_router::<ServerContext>())
        .with_state(Arc::clone(resources))
}

/// An administrator owning a tenant, and a bearer token for them.
async fn administrator(resources: &Arc<ServerContext>) -> (Uuid, TenantId, String) {
    let mut user = User::new(
        "agent-admin@example.com".to_owned(),
        "unused-hash".to_owned(),
        Some("Agent Admin".to_owned()),
    );
    user.is_admin = true;
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Agent admin tenant".to_owned(),
            slug: format!("agent-admin-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    (user.id, tenant_id, format!("Bearer {token}"))
}

/// An athlete who is a member of `tenant_id`.
async fn athlete_in(resources: &Arc<ServerContext>, tenant_id: TenantId) -> Uuid {
    let user = User::new(
        "assigned-athlete@example.com".to_owned(),
        "unused-hash".to_owned(),
        Some("Assigned Athlete".to_owned()),
    );
    resources.common.repos.users.create(&user).await.unwrap();
    let now = Utc::now();
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(INSERT_MEMBER)
                .bind(Uuid::new_v4().to_string())
                .bind(tenant_id.to_string())
                .bind(user.id.to_string())
                .bind(now.to_rfc3339())
                .bind(now.to_rfc3339())
                .execute(db.pool())
                .await
                .unwrap();
        }
        // `tenant_users` keys are `uuid` columns and the timestamps are
        // `timestamptz` on PostgreSQL, so the binds carry the native types.
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(INSERT_MEMBER)
                .bind(Uuid::new_v4())
                .bind(tenant_id.as_uuid())
                .bind(user.id)
                .bind(now)
                .bind(now)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
    user.id
}

/// The athlete's `coach`-category notifications once `expected` have landed,
/// or whatever is there when the dispatch had its time.
async fn coach_notifications(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    expected: usize,
) -> Vec<Notification> {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the server boots with its notification service");
    let mut rows = Vec::new();
    for _ in 0..40 {
        (rows, _, _) = service
            .list_notifications(
                user_id,
                CommereTenantId(tenant_id.as_uuid()),
                20,
                0,
                Some("coach"),
                false,
            )
            .await
            .unwrap();
        if rows.len() >= expected {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    rows
}

fn of_type<'a>(rows: &'a [Notification], wire: &str) -> Vec<&'a Notification> {
    rows.iter()
        .filter(|row| row.notification_type == wire)
        .collect()
}

#[tokio::test]
async fn an_assignment_and_an_edit_each_say_what_the_administrator_did() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin_id, tenant_id, auth) = administrator(&resources).await;
    let athlete_id = athlete_in(&resources, tenant_id).await;
    let agent = resources
        .common
        .repos
        .agents
        .create_system_agent(
            admin_id,
            tenant_id,
            &CreateSystemAgentRequest {
                title: AGENT_TITLE.to_owned(),
                description: Some("Assigned, then edited".to_owned()),
                system_prompt: "You answer marathon questions.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap();
    let assign_path = format!("/api/admin/agents/{}/assign", agent.id);
    let assignment = json!({ "user_ids": [athlete_id.to_string()] });

    // The assignment: the athlete is told the agent was added, in those words.
    let response = AxumTestRequest::post(&assign_path)
        .header("authorization", &auth)
        .json(&assignment)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["assigned_count"], 1, "{body}");

    let rows = coach_notifications(&resources, athlete_id, tenant_id, 1).await;
    let assigned = of_type(&rows, "agent_assigned");
    assert_eq!(assigned.len(), 1, "{rows:?}");
    assert_eq!(assigned[0].title, "Nouvel agent");
    assert_eq!(
        assigned[0].body,
        format!("Un administrateur a ajouté {AGENT_TITLE} à tes agents.")
    );
    assert!(
        assigned[0]
            .data
            .as_ref()
            .is_some_and(|data| data.get("screen").is_none()),
        "no thread with the agent exists yet, so the notice opens nothing: {:?}",
        assigned[0].data
    );

    // The same assignment again changes nothing, so it tells nobody.
    let response = AxumTestRequest::post(&assign_path)
        .header("authorization", &auth)
        .json(&assignment)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["assigned_count"], 0, "{body}");
    sleep(Duration::from_millis(400)).await;
    let rows = coach_notifications(&resources, athlete_id, tenant_id, 1).await;
    assert_eq!(
        of_type(&rows, "agent_assigned").len(),
        1,
        "a repeated assignment raises no second notice: {rows:?}"
    );

    // The edit: the assigned athlete hears the agent changed — not their plan.
    let response = AxumTestRequest::put(&format!("/api/admin/agents/{}", agent.id))
        .header("authorization", &auth)
        .json(&json!({ "title": EDITED_TITLE }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);

    let rows = coach_notifications(&resources, athlete_id, tenant_id, 2).await;
    let updated = of_type(&rows, "agent_updated");
    assert_eq!(updated.len(), 1, "{rows:?}");
    assert_eq!(updated[0].title, "Agent mis à jour");
    assert_eq!(
        updated[0].body,
        format!("Un administrateur a mis à jour {EDITED_TITLE}.")
    );
    assert!(
        rows.iter()
            .all(|row| row.notification_type != "plan_updated"),
        "no plan changed, so none is claimed: {rows:?}"
    );
}
