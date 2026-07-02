// ABOUTME: End-to-end Guardian enforcement through the real ServerContext + UniversalExecutor chokepoint.
// ABOUTME: Deterministic (no LLM): enforce + zero destructive budget denies an IRREVERSIBLE tool before it runs; reads pass.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian enforcement e2e.
//!
//! Drives a real `ServerContext`-backed `UniversalExecutor` (the one true
//! dispatch chokepoint every transport funnels through) in `enforce` mode and
//! asserts the runtime Guardian blocks a consequential tool while leaving reads
//! untouched. Single test fn so the process-wide Guardian policy singleton
//! initializes once, after the env is set.

mod common;

use std::env;

use common::{create_test_server_resources, create_test_user};
use pierre_core::models::TenantId;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

fn request(tool: &str, args: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: args,
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

fn guardian_error_code(response: &UniversalResponse) -> Option<String> {
    response
        .result
        .as_ref()
        .and_then(|r| r.get("error_code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[tokio::test]
async fn guardian_enforce_chokepoint_denies_irreversible_allows_reads_e2e() {
    // Arrange enforce mode with a zero destructive budget BEFORE the Guardian
    // policy singleton is first read (it initializes on the first tool dispatch).
    // Set before the Guardian policy singleton is first read (single-threaded
    // setup in this test's own process, before any tool dispatch).
    env::set_var("GUARDIAN_MODE", "enforce");
    env::set_var("GUARDIAN_MAX_DESTRUCTIVE_PER_TURN", "0");

    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    // One executor = one turn; its turn token is the Guardian's per-turn key.
    let executor =
        UniversalToolExecutor::new(resources.clone()).with_turn_token("e2e-turn".to_owned());

    // (a) An IRREVERSIBLE tool is denied at the chokepoint by the zero budget —
    //     before the tool body ever runs. `disconnect_provider` is classified
    //     IRREVERSIBLE in the registry.
    let deny = executor
        .execute_tool(request(
            "disconnect_provider",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");
    assert!(
        !deny.success,
        "an irreversible tool must be denied under enforce + zero destructive budget"
    );
    assert_eq!(
        guardian_error_code(&deny).as_deref(),
        Some("guardian_denied"),
        "denied response must carry the guardian_denied machine code"
    );
    assert_eq!(
        deny.result
            .as_ref()
            .and_then(|r| r.get("reason"))
            .and_then(Value::as_str),
        Some("budget_exceeded"),
        "the denial reason is the per-turn budget"
    );

    // (b) A read tool is NOT guardian-denied — enforce must not break ordinary
    //     reads. (It may still fail for lack of provider data; that's fine, as
    //     long as it is not a Guardian denial.)
    let read = executor
        .execute_tool(request(
            "get_athlete",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");
    assert_ne!(
        guardian_error_code(&read).as_deref(),
        Some("guardian_denied"),
        "a read tool must never be guardian-denied"
    );
}
