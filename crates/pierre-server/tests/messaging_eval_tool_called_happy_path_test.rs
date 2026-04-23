// ABOUTME: Happy-path integration for assert_tool_called against a seeded llm_usage summary row
// ABOUTME: Proves the asserter reads tools_called correctly when a tool was actually invoked
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Positive-mode integration for
//! [`assert_tool_called`](helpers::messaging_eval::assert_tool_called).
//!
//! The sibling `messaging_eval_phase_1_integration_test.rs` drives the
//! full webhook → pipeline → endpoint round trip but the `MockLlmProvider`
//! used there never emits `tool_calls`, so `tools_called` stays empty
//! and the asserter is only exercised in inverted mode. Setting up a
//! stub tool registry to route a real tool-call through the full
//! pipeline is disproportionate scope for the spike (it would require
//! an injection point on `create_test_server_resources_with_llm` plus
//! a no-auth stub tool registered under a known name).
//!
//! Instead, this test bypasses the pipeline and seeds an `llm_usage`
//! summary row directly. The goal is narrow: prove that when a
//! conversation turn has `tools_called = ["get_activities"]` on its
//! summary row, the `/internal/conversation-turn/{turn_id}` endpoint
//! serializes it into the response JSON, and
//! [`assert_tool_called`] returns `Ok` for a matching tool name and
//! `Err` for an absent one. Together with the unit-test happy path
//! and the phase-1 inverted-mode integration, this closes the coverage
//! triangle on the asserter.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod tool_called_happy_path {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::messaging_eval::assert_tool_called;
    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_core::models::usage::{InsertLlmUsage, TURN_SUMMARY_CALL_TYPE};
    use pierre_core::models::ConversationTurnId;
    use pierre_mcp_server::mcp::resources::ServerResources;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::permissions::UserRole;
    use pierre_mcp_server::routes::llm_consumption::LlmConsumptionRoutes;
    use serde_json::Value;
    use serial_test::serial;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    async fn create_admin_user_in_tenant(
        resources: &Arc<ServerResources>,
        email: &str,
    ) -> (Uuid, TenantId, String) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Tool-Called Admin".to_owned()),
        );
        user.is_admin = true;
        user.role = UserRole::Admin;
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Tool-Called Tenant {email}"),
            slug: format!("tool-called-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources.repos.tenants.create(&tenant).await.unwrap();
        resources
            .repos
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .unwrap();

        let token = resources
            .auth_manager
            .generate_token(&user, &resources.jwks_manager)
            .unwrap();
        (user_id, tenant_id, format!("Bearer {token}"))
    }

    /// Seed an `llm_usage` summary row so the
    /// `/internal/conversation-turn/{turn_id}` endpoint has something
    /// to return, bypassing the pipeline.
    async fn seed_turn_summary_with_tools(
        resources: &Arc<ServerResources>,
        tenant_id: TenantId,
        user_id: Uuid,
        turn: ConversationTurnId,
        tools_called_json: &str,
    ) {
        let tenant_str = tenant_id.to_string();
        let user_str = user_id.to_string();
        let params = InsertLlmUsage {
            tenant_id: &tenant_str,
            user_id: &user_str,
            conversation_id: None,
            turn_id: turn,
            provider: "mock",
            model: "mock-model",
            prompt_tokens: 42,
            completion_tokens: 11,
            total_tokens: 53,
            call_type: TURN_SUMMARY_CALL_TYPE,
            tool_calls_count: 1,
            tools_called: tools_called_json,
            execution_time_ms: Some(2_400),
        };
        resources
            .repos
            .llm_usage
            .insert_llm_usage(&params)
            .await
            .expect("seed llm_usage summary row");
    }

    /// Query the admin-only conversation-turn endpoint and return the
    /// parsed body, asserting a 200 response.
    async fn fetch_turn_json(
        resources: &Arc<ServerResources>,
        admin_auth: &str,
        turn: ConversationTurnId,
    ) -> Value {
        let router = LlmConsumptionRoutes::routes(Arc::clone(resources));
        let resp = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn}"))
            .header("authorization", admin_auth)
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        resp.json::<Value>()
    }

    #[tokio::test]
    #[serial]
    async fn assert_tool_called_passes_when_summary_row_lists_the_tool() {
        let resources = create_test_server_resources().await.unwrap();
        let (admin_user_id, tenant_id, admin_auth) =
            create_admin_user_in_tenant(&resources, "tool-called-pos@example.com").await;

        let turn = ConversationTurnId::new();
        seed_turn_summary_with_tools(
            &resources,
            tenant_id,
            admin_user_id,
            turn,
            r#"["get_activities","get_training_load"]"#,
        )
        .await;

        let body = fetch_turn_json(&resources, &admin_auth, turn).await;

        // The happy path we couldn't exercise through the pipeline in the
        // phase-1 integration test: asserter returns Ok for a tool that
        // is in the summary row's tools_called array.
        assert_tool_called(&body, "get_activities")
            .expect("get_activities must be grounded in the seeded summary row");
        assert_tool_called(&body, "get_training_load")
            .expect("get_training_load must also be grounded");
    }

    #[tokio::test]
    #[serial]
    async fn assert_tool_called_fails_when_tool_absent_from_summary_row() {
        let resources = create_test_server_resources().await.unwrap();
        let (admin_user_id, tenant_id, admin_auth) =
            create_admin_user_in_tenant(&resources, "tool-called-neg@example.com").await;

        let turn = ConversationTurnId::new();
        seed_turn_summary_with_tools(
            &resources,
            tenant_id,
            admin_user_id,
            turn,
            r#"["get_activities"]"#,
        )
        .await;

        let body = fetch_turn_json(&resources, &admin_auth, turn).await;

        // Inverted mode on a real endpoint response: the asserter must
        // flag a missing tool even when other tools were invoked. This
        // is the hallucination-catching behavior in real scenarios.
        let err = assert_tool_called(&body, "suggest_goals")
            .expect_err("suggest_goals was not in the summary row; asserter must flag");
        assert_eq!(err.expected, "suggest_goals");
        assert_eq!(err.actual, vec!["get_activities".to_owned()]);
    }
}
