// ABOUTME: The dispatch chokepoint refuses REQUIRES_PROVIDER tools for providerless users — in-band
// ABOUTME: Both directions pinned: providerless refused with recovery metadata, connected reaches the body
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The barrier that replaces the conversation gate
//!
//! Phase 5 opens the coach conversation to providerless users. Before it can,
//! something must still stand between them and the data tools, because the old
//! barrier was the conversation gate itself: `onboarding_gate.rs` claimed an
//! "MCP tool execution gate" in its ABOUTME, but no such call site existed —
//! ~20 of 103 tools refused in their own bodies via the provider resolvers,
//! and the rest served quiet empty shapes a model reads as "nothing recent".
//!
//! The chokepoint in `UniversalExecutor::authorization_refusal` is that
//! barrier. These tests pin both directions, because a gate that always
//! refuses would pass the providerless half and still be broken: the refusal
//! must fire on zero connections and must NOT fire on one.
//!
//! Env-touching tests share `#[serial_test::serial]` with the others in this
//! file: `PIERRE_DEFAULT_PROVIDER` is process-global, and a parallel test
//! observing a half-set environment would flake.

mod common;

use common::{create_test_server_resources, create_test_tenant, create_test_tenant_with_provider};
use pierre_core::models::UserOAuthToken;
use pierre_tool_runtime::protocol::{
    auth_required_provider, UniversalRequest, UniversalResponse, UniversalToolExecutor,
    META_AUTH_REQUIRED_PROVIDER,
};
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;
use std::env;
use uuid::Uuid;

/// `refresh_provider_data` declares `REQUIRES_PROVIDER` in its capabilities,
/// which is what routes it through the chokepoint.
const COVERED_TOOL: &str = "refresh_provider_data";

fn request(user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: COVERED_TOOL.to_owned(),
        parameters: json!({"provider": "strava", "wait": false}),
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// A providerless dispatch of a covered tool is refused in-band, with the
/// metadata the auth-recovery flow keys on.
#[tokio::test]
#[serial]
async fn a_providerless_dispatch_is_refused_with_recovery_metadata() {
    env::remove_var("PIERRE_DEFAULT_PROVIDER");
    let resources = create_test_server_resources().await.unwrap();
    let (user, _token) = create_test_tenant(&resources, "chokepoint-none@test.local")
        .await
        .unwrap();
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let tenant_id = tenants.first().unwrap().id.to_string();

    let executor = UniversalToolExecutor::new(resources);
    let resp = executor
        .execute_tool(request(user.id, &tenant_id))
        .await
        .expect("an in-band refusal, not a protocol error — the turn must not be abandoned");

    assert!(
        !resp.success,
        "zero provider_connections rows must refuse a REQUIRES_PROVIDER dispatch"
    );
    assert_eq!(
        auth_required_provider(&resp).as_deref(),
        Some("sciotte"),
        "the refusal must carry the exact metadata the auth_recovery stage decodes \
         into a hosted-login flow; anything else silently breaks that handoff"
    );
    assert!(
        resp.error
            .as_deref()
            .is_some_and(|e| e.contains("No fitness provider connected")),
        "the error copy is the canonical one minted by no_provider_refusal, got: {:?}",
        resp.error
    );
}

/// A connected dispatch of the same tool must get PAST the chokepoint.
///
/// Asserted on the refusal shape rather than on `success`: the body may still
/// fail for its own reasons in a test environment (no live OAuth token), but it
/// must never fail with the chokepoint's "no provider connected" refusal —
/// otherwise the gate has regressed to always-refuse while every providerless
/// test stays green.
#[tokio::test]
#[serial]
async fn a_connected_dispatch_reaches_the_tool_body() {
    env::remove_var("PIERRE_DEFAULT_PROVIDER");
    let resources = create_test_server_resources().await.unwrap();
    let (user, _token) =
        create_test_tenant_with_provider(&resources, "chokepoint-strava@test.local")
            .await
            .unwrap();
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let tenant_id = tenants.first().unwrap().id.to_string();

    let executor = UniversalToolExecutor::new(resources);
    let resp = executor
        .execute_tool(request(user.id, &tenant_id))
        .await
        .expect("dispatch proceeds");

    assert!(
        !resp
            .error
            .as_deref()
            .is_some_and(|e| e.contains("No fitness provider connected")),
        "a user with a provider_connections row must never see the chokepoint refusal, got: {:?}",
        resp.error
    );
}

/// A live OAuth token with **no** `provider_connections` row must still reach
/// the tool body.
///
/// The two tables are written by different paths and are known to drift. The
/// chokepoint originally read only `provider_connections`, which meant an
/// athlete holding a working token but missing their connection row was refused
/// from every provider-backed tool — surfacing as "the coach stopped seeing my
/// data", a worse failure than the hallucination the gate exists to prevent.
///
/// Refusing requires *both* tables to come up empty, and this pins that: delete
/// the `oauth_tokens` half of the check and this test fails while every
/// providerless test above stays green.
#[tokio::test]
#[serial]
async fn a_token_without_a_connection_row_is_not_refused() {
    env::remove_var("PIERRE_DEFAULT_PROVIDER");
    let resources = create_test_server_resources().await.unwrap();
    let (user, _token) = create_test_tenant(&resources, "chokepoint-drift@test.local")
        .await
        .unwrap();
    let repos = resources.coach.database.repositories();
    let tenants = repos.tenants.list_for_user(user.id).await.unwrap();
    let tenant_id = tenants.first().unwrap().id;

    // A token but deliberately no `provider_connections` row — the drift shape.
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken::new(
            user.id,
            tenant_id.to_string(),
            "strava".to_owned(),
            "drift_access_token".to_owned(),
            Some("drift_refresh_token".to_owned()),
            Some(chrono::Utc::now() + chrono::Duration::hours(6)),
            Some("activity:read_all".to_owned()),
        ))
        .await
        .unwrap();
    assert!(
        repos
            .provider_connections
            .get_for_user(user.id, None)
            .await
            .unwrap()
            .is_empty(),
        "the drift shape requires an empty provider_connections — otherwise this \
         test would pass for the wrong reason"
    );

    let executor = UniversalToolExecutor::new(resources);
    let resp = executor
        .execute_tool(request(user.id, &tenant_id.to_string()))
        .await
        .expect("dispatch proceeds");

    assert!(
        !resp
            .error
            .as_deref()
            .is_some_and(|e| e.contains("No fitness provider connected")),
        "an athlete holding a live OAuth token must never see the chokepoint \
         refusal just because their connection row is missing, got: {:?}",
        resp.error
    );
}

/// A request naming a **non-OAuth** provider is not refused, even with nothing
/// connected.
///
/// `synthetic` generates its data and holds no credential — which is why
/// `fetch_provider_activities` carries an explicit `requires_oauth == false`
/// branch for it. Demo and seeded accounts run entirely on that provider, so a
/// chokepoint that refused on "no connection rows" alone would lock them out of
/// every analytics tool while their data was sitting right there.
#[tokio::test]
#[serial]
async fn a_named_non_oauth_provider_is_not_refused() {
    env::remove_var("PIERRE_DEFAULT_PROVIDER");
    let resources = create_test_server_resources().await.unwrap();
    let (user, _token) = create_test_tenant(&resources, "chokepoint-synthetic@test.local")
        .await
        .unwrap();
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let tenant_id = tenants.first().unwrap().id.to_string();

    let executor = UniversalToolExecutor::new(resources);

    for provider in ["synthetic", "synthetic_sleep"] {
        let mut req = request(user.id, &tenant_id);
        req.parameters = json!({"provider": provider, "wait": false});
        let resp = executor.execute_tool(req).await.expect("dispatch proceeds");

        assert!(
            !resp
                .error
                .as_deref()
                .is_some_and(|e| e.contains("No fitness provider connected")),
            "provider={provider} needs no credential and must never be refused \
             for lack of one, got: {:?}",
            resp.error
        );
    }
}

/// An **unregistered** provider name does not buy a way past the gate.
///
/// `requires_oauth` answers `false` for any name the registry does not know, so
/// a credential-free-provider bypass keyed on that alone would wave through the
/// `"all"` sentinel and every provider name an LLM invents — turning a one-word
/// argument into a general bypass of the refusal. Only names the registry
/// actually carries may stand aside.
#[tokio::test]
#[serial]
async fn an_unregistered_provider_name_does_not_bypass_the_refusal() {
    env::remove_var("PIERRE_DEFAULT_PROVIDER");
    let resources = create_test_server_resources().await.unwrap();
    let (user, _token) = create_test_tenant(&resources, "chokepoint-bogus@test.local")
        .await
        .unwrap();
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let tenant_id = tenants.first().unwrap().id.to_string();
    let executor = UniversalToolExecutor::new(resources);

    for bogus in ["all", "not_a_real_provider"] {
        let mut req = request(user.id, &tenant_id);
        req.parameters = json!({"provider": bogus, "wait": false});
        let resp = executor.execute_tool(req).await.expect("dispatch proceeds");
        assert!(
            !resp.success,
            "provider={bogus} must not reach a success shape for a providerless \
             athlete; got {resp:?}"
        );
        assert_eq!(
            auth_required_provider(&resp).as_deref(),
            Some("sciotte"),
            "provider={bogus} must still carry the recovery metadata"
        );
    }
}

/// `PIERRE_DEFAULT_PROVIDER` short-circuits the chokepoint, mirroring both
/// resolvers, which serve from the override without ever consulting
/// `provider_connections`. A chokepoint that ignored the override would refuse
/// users those resolvers serve today — a behaviour change on every deployment
/// that sets it.
#[tokio::test]
#[serial]
async fn the_default_provider_override_bypasses_the_chokepoint() {
    env::set_var("PIERRE_DEFAULT_PROVIDER", "strava");
    let resources = create_test_server_resources().await.unwrap();
    let (user, _token) = create_test_tenant(&resources, "chokepoint-env@test.local")
        .await
        .unwrap();
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let tenant_id = tenants.first().unwrap().id.to_string();

    let executor = UniversalToolExecutor::new(resources);
    let resp = executor.execute_tool(request(user.id, &tenant_id)).await;
    env::remove_var("PIERRE_DEFAULT_PROVIDER");

    let resp = resp.expect("dispatch proceeds under the override");
    assert!(
        !resp
            .error
            .as_deref()
            .is_some_and(|e| e.contains("No fitness provider connected")),
        "with the deployment override set, the chokepoint must stand aside exactly \
         as the resolvers do, got: {:?}",
        resp.error
    );
}

/// The reconnect signal is a metadata key holding a **string**, and every
/// consumer decodes it the same way through `auth_required_provider`.
///
/// Read by hand at six sites, the check was written three different ways: two
/// only asked whether the key was present. A non-string value there would have
/// read as "this athlete must reconnect" and sent the turn down the hosted-login
/// path with no provider to name, so the shared decoder rejects it.
#[test]
fn auth_required_provider_reads_only_a_string_slug() {
    let no_metadata = UniversalResponse {
        success: false,
        result: None,
        error: Some("boom".to_owned()),
        metadata: None,
    };
    assert_eq!(
        auth_required_provider(&no_metadata),
        None,
        "a failure carrying no metadata names no provider"
    );

    let unrelated_metadata = UniversalResponse {
        metadata: Some(HashMap::from([("elapsed_ms".to_owned(), json!(12))])),
        ..no_metadata.clone()
    };
    assert_eq!(
        auth_required_provider(&unrelated_metadata),
        None,
        "metadata without the key names no provider"
    );

    let non_string = UniversalResponse {
        metadata: Some(HashMap::from([(
            META_AUTH_REQUIRED_PROVIDER.to_owned(),
            json!(true),
        )])),
        ..no_metadata.clone()
    };
    assert_eq!(
        auth_required_provider(&non_string),
        None,
        "the key holding a non-string must not read as a reconnect signal"
    );

    let dead_connection = UniversalResponse {
        metadata: Some(HashMap::from([(
            META_AUTH_REQUIRED_PROVIDER.to_owned(),
            json!("strava"),
        )])),
        ..no_metadata
    };
    assert_eq!(
        auth_required_provider(&dead_connection).as_deref(),
        Some("strava"),
        "the slug the athlete must reconnect is returned verbatim"
    );
}
