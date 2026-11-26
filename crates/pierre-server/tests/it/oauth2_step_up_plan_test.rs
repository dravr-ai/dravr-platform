// ABOUTME: Slice 4b's remaining plan tests: a re-consented parent ends its child, total_rows over /mcp, introspection's step_up
// ABOUTME: And a dispatch nested in the call that consumed a single-use child runs under it, once per top-level call

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The tests plan revision 16 (R-N1) names that `oauth2_step_up_test` did
//! not carry (owl-parity LANDING item 14), and the nested-dispatch case of a
//! single-use child: consumed once per top-level call, so a tool that
//! dispatches another tool inside that call is not refused `grant_consumed`.

use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use crate::oauth2_grant_details_test::{app, exchange, tokens, world, world_with, Client, World};
use crate::oauth2_step_up_test::{
    allow_health_records, children, events, file_a_lab, read_records, stepped_up_in,
    stepped_up_under, stepped_up_with_code,
};

use base64::{engine::general_purpose, Engine as _};
use dravr_barrage::Stream;
use dravr_tronc::dates::Zone;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_core::models::{GrantApproval, GrantEvent, OAuthClientGrant};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_services::usage_counter::{current_counter, increment_counter, LIFETIME_COUNTER_PREFIX};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

/// The athlete's live parent grant `id`.
async fn parent_row(world: &World, id: &str) -> OAuthClientGrant {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .list_client_grants(&world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .find(|grant| grant.id == id)
        .expect("the parent stands")
}

// ============================================================================
// A re-consented parent ends its child
// ============================================================================

/// The athlete approves the same app again for the same scope and details:
/// the parent's row is kept and re-consented in place, and the one-off
/// question approved under the old consent ends with an `expire` or
/// `revoke` receipt.
#[tokio::test]
async fn a_parent_re_consented_in_place_ends_its_child() {
    let (world, _client, parent, child, _code) = stepped_up_with_code().await;
    let row = parent_row(&world, &parent).await;
    let stored = world
        .resources
        .common
        .repos
        .oauth2_server
        .store_client_grant(
            &OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                consent_version: Some("re-consented".to_owned()),
                ..row
            },
            &GrantApproval::by_user(),
        )
        .await
        .unwrap();
    assert_eq!(
        stored, parent,
        "premise: the re-consent kept the parent's row"
    );
    assert!(
        children(&world).await.is_empty(),
        "bug: a re-consented parent left its step-up child live"
    );
    let receipts = events(&world, &child).await;
    assert_eq!(
        receipts.len(),
        2,
        "bug: the ended child has no terminal receipt: {receipts:?}"
    );
    assert!(
        receipts[1].is_terminal(),
        "bug: the ended child's last receipt is not terminal: {receipts:?}"
    );
}

/// A re-consent to other details replaces the parent: the old row and its
/// child end, the new row has none.
#[tokio::test]
async fn a_parent_replaced_by_a_re_consent_ends_its_child() {
    let (world, _client, parent, _child, _code) = stepped_up_with_code().await;
    let row = parent_row(&world, &parent).await;
    let replaced = world
        .resources
        .common
        .repos
        .oauth2_server
        .replace_client_grant(
            &parent,
            &OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                ..row
            },
            &GrantApproval::by_user(),
        )
        .await
        .unwrap();
    assert_ne!(replaced, parent, "premise: the replace wrote a new row");
    assert!(
        children(&world).await.is_empty(),
        "bug: a replaced parent left its step-up child live"
    );
}

// ============================================================================
// total_rows over /mcp
// ============================================================================

/// The child's lifetime row budget, spent: its read over `/mcp` is refused
/// `health_budget_exhausted`, before it reads, and the child is not consumed.
#[tokio::test]
async fn a_child_whose_total_rows_are_spent_is_refused_over_mcp() {
    common::init_server_config();
    let (world, client, _parent, child, code) = stepped_up_under(&json!({ "total_rows": 1 })).await;
    file_a_lab(&world).await;
    allow_health_records(&world).await;
    let (token, _) = tokens(&exchange(&world, &client, &code).await);
    let counter = format!("{LIFETIME_COUNTER_PREFIX}grant_rows:{child}");
    increment_counter(
        world.resources.common.repos.usage_counters.as_ref(),
        &world.tenant_id,
        &world.user.id.to_string(),
        &counter,
        1,
    )
    .await
    .unwrap();

    let refused = read_records(&world, &token, "labs", Some("T2")).await;
    let text = refused.to_string();
    assert!(
        text.contains("health_budget_exhausted") && !text.contains("ferritin"),
        "bug: a child past its total rows read over /mcp: {refused}"
    );
    assert!(
        !text.contains("grant_consumed"),
        "premise: the refusal is the budget's, not the single use's: {refused}"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp],
        "bug: a call refused on its total rows consumed the child"
    );
}

/// The rows a child's one read serves over `/mcp` are levied on its
/// lifetime counter.
#[tokio::test]
async fn a_childs_read_over_mcp_levies_its_total_rows() {
    common::init_server_config();
    let (world, client, _parent, child, code) =
        stepped_up_under(&json!({ "total_rows": 50 })).await;
    file_a_lab(&world).await;
    allow_health_records(&world).await;
    let (token, _) = tokens(&exchange(&world, &client, &code).await);
    let labs = read_records(&world, &token, "labs", Some("T2")).await;
    assert!(
        labs.to_string().contains("ferritin"),
        "premise: the child's read was served: {labs}"
    );
    let counter = format!("{LIFETIME_COUNTER_PREFIX}grant_rows:{child}");
    let levied = current_counter(
        world.resources.common.repos.usage_counters.as_ref(),
        &world.tenant_id,
        &world.user.id.to_string(),
        &counter,
    )
    .await
    .unwrap();
    assert!(
        levied >= 1,
        "bug: the rows the child served were not levied on its total: {levied}"
    );
}

// ============================================================================
// Introspection echoes step_up
// ============================================================================

/// A resource server introspecting the parent's token while the child is
/// live reads the child's step-up; once consumed, none.
#[tokio::test]
async fn introspection_echoes_the_live_childs_step_up_end_to_end() {
    common::init_server_config();
    let introspector_id = format!("resource-server-{}", Uuid::new_v4().simple());
    let mut config = common::test_server_config();
    config.oauth2_server.introspection_clients = vec![introspector_id.clone()];
    let world = world_with(config).await;
    let secret = ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
        .register_client_credentials_client(&introspector_id, "Resource server")
        .await
        .unwrap();
    let introspector = Client {
        id: introspector_id,
        secret,
    };
    let (world, client, _parent, child, code) = stepped_up_in(world, &Value::Null).await;
    let (token, _) = tokens(&exchange(&world, &client, &code).await);

    let introspect = |token: String| {
        let basic = general_purpose::STANDARD
            .encode(format!("{}:{}", introspector.id, introspector.secret));
        let router = app(&world.resources);
        async move {
            let response = AxumTestRequest::post("/oauth2/introspect")
                .header("authorization", &format!("Basic {basic}"))
                .form(&[
                    ("token", token.as_str()),
                    ("token_type_hint", "access_token"),
                ])
                .send(router)
                .await;
            assert_eq!(response.status(), 200, "{}", response.body_text());
            serde_json::from_str::<Value>(&response.body_text()).unwrap()
        }
    };
    let answer = introspect(token.clone()).await;
    assert_eq!(
        answer["active"], true,
        "premise: the token is live: {answer}"
    );
    let step_up = &answer["step_up"];
    assert_eq!(
        step_up["grant_id"], child,
        "bug: introspection does not name the live child: {answer}"
    );
    assert!(
        step_up["step_up_result"].is_object() && step_up["host_profile"].is_object(),
        "bug: introspection's step_up lacks its result or host profile: {answer}"
    );

    // Consumed the way a client consumes it: its one read over /mcp.
    file_a_lab(&world).await;
    allow_health_records(&world).await;
    let labs = read_records(&world, &token, "labs", Some("T2")).await;
    assert!(
        labs.to_string().contains("ferritin"),
        "premise: the child's read was served: {labs}"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp, GrantEvent::Consume],
        "premise: the read consumed the child"
    );
    let after = introspect(token).await;
    assert!(
        after.get("step_up").is_none_or(Value::is_null),
        "bug: introspection echoes a consumed child: {after}"
    );
}

// ============================================================================
// A dispatch nested in the call that consumed a single-use child
// ============================================================================

/// `analyze_activity` dispatches `get_activity_intelligence` through the
/// registry inside its own call. Under a single-use ceiling the outer call
/// consumes the grant once; the nested dispatch runs under the grant its
/// call spent rather than being refused `grant_consumed`, and a second
/// top-level call is still refused.
#[tokio::test]
async fn a_dispatch_nested_in_the_consuming_call_is_not_refused_grant_consumed() {
    common::init_server_config();
    let (world, _client, _parent, child, _code) = stepped_up_in(world().await, &Value::Null).await;
    let stored = children(&world)
        .await
        .into_iter()
        .find(|row| row.grant.id == child)
        .unwrap();
    // The child's own ceiling, widened to what analyze_activity and the tool
    // it nests read, so the call runs under the single-use grant itself.
    let mut ceiling = GrantCeiling::from_document(
        &child,
        stored.grant.authorization_details.as_deref().unwrap(),
        &Zone::utc(),
    )
    .unwrap();
    assert!(ceiling.single_use, "premise: a one-off child is single-use");
    ceiling.streams = vec![Stream::Workout, Stream::WorkoutRaw];
    let call = || async {
        let executor = UniversalToolExecutor::new(world.resources.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_ceiling(Some(ceiling.clone()))
            .with_transport(Transport::McpHttp);
        executor
            .execute_tool(UniversalRequest {
                tool_name: "analyze_activity".to_owned(),
                parameters: json!({"provider": "synthetic", "activity_id": "nested-1"}),
                user_id: world.user.id.to_string(),
                protocol: "mcp".to_owned(),
                tenant_id: Some(world.tenant_id.clone()),
            })
            .await
    };

    let first = format!("{:?}", call().await);
    assert!(
        !first.contains("grant_consumed"),
        "bug: the dispatch nested in the call that consumed the child was refused: {first}"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp, GrantEvent::Consume],
        "bug: the call did not consume the child exactly once"
    );
    let second = format!("{:?}", call().await);
    assert!(
        second.contains("grant_consumed"),
        "bug: a second top-level call under the consumed child was not refused: {second}"
    );
}
