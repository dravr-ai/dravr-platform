// ABOUTME: Proves the OAuth scope gate refuses a tool the caller's grant does not cover
// ABOUTME: Covers the derivation, the dispatch chokepoint, and that a sufficient grant still passes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The scope gate, end to end at the dispatch chokepoint.
//!
//! Every transport funnels through `UniversalToolExecutor::execute_tool`, so
//! exercising the executor with a narrowed grant is exercising the gate every
//! caller meets. The refusal tests are paired with a sufficient-grant test on
//! the *same* tool: a gate that refuses everything would satisfy the refusals
//! alone, and that is the failure mode a scope check is most likely to have.

mod common;

use common::{create_test_server_resources, create_test_user};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::FunctionCall;
use pierre_tool_runtime::function_dispatch::execute_function_calls;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::protocols::ProtocolError;
use pierre_tool_runtime::scopes::{missing_scope, required_scopes};
use serde_json::{json, Value};
use uuid::Uuid;

use dravr_tronc::mcp::tool::ToolCapabilities;

fn request(tool: &str, args: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: args,
        user_id: user_id.to_string(),
        protocol: "mcp".to_owned(),
        tenant_id: Some(tenant.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// The derivation names the scope the flags describe — concrete pairs, not a
/// smoke test. A mapping that returned an empty requirement for everything
/// would make every gate below pass vacuously, so it is pinned first.
#[test]
fn capability_flags_derive_their_scope() {
    assert_eq!(
        required_scopes(ToolCapabilities::READS_DATA),
        vec![OAuthScope::FitnessRead]
    );
    assert_eq!(
        required_scopes(ToolCapabilities::WRITES_DATA),
        vec![OAuthScope::FitnessWrite]
    );

    // PROFILE moves the read/write to the identity half of the vocabulary
    // rather than adding a third requirement — that is the whole point of the
    // split, so it is asserted as an exact set, not a `contains`.
    assert_eq!(
        required_scopes(ToolCapabilities::READS_DATA | ToolCapabilities::PROFILE),
        vec![OAuthScope::ProfileRead]
    );
    assert_eq!(
        required_scopes(ToolCapabilities::WRITES_DATA | ToolCapabilities::PROFILE),
        vec![OAuthScope::ProfileWrite]
    );

    // A tool that reads and writes needs both.
    assert_eq!(
        required_scopes(ToolCapabilities::READS_DATA | ToolCapabilities::WRITES_DATA),
        vec![OAuthScope::FitnessRead, OAuthScope::FitnessWrite]
    );

    // Runtime requirements are not permissions and must not become scopes: a
    // vocabulary that published `requires_tenant` would ask an athlete to
    // consent to an implementation detail.
    assert!(required_scopes(
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
    )
    .is_empty());
}

/// The missing scope is *named*, because RFC 6750 §3.1 requires the challenge
/// to say which grant was needed. A bool would leave the transport to recompute
/// it, and a challenge assembled twice can disagree with its own refusal.
#[test]
fn the_missing_scope_is_named_not_just_detected() {
    let read_only = [OAuthScope::FitnessRead];

    assert_eq!(
        missing_scope(&read_only, ToolCapabilities::WRITES_DATA),
        Some(OAuthScope::FitnessWrite)
    );
    assert_eq!(
        missing_scope(
            &read_only,
            ToolCapabilities::READS_DATA | ToolCapabilities::PROFILE
        ),
        Some(OAuthScope::ProfileRead),
        "fitness:read must not satisfy a profile read — that is the split"
    );
    assert_eq!(
        missing_scope(&read_only, ToolCapabilities::READS_DATA),
        None,
        "a grant that covers the tool yields no missing scope"
    );
    assert_eq!(
        missing_scope(&OAuthScope::self_grant(), ToolCapabilities::ADMIN_ONLY),
        None,
        "the self grant covers every scope; the ROLE gate is what refuses admin"
    );
}

/// A write tool is refused when the grant is read-only, at the chokepoint every
/// transport passes.
#[tokio::test]
async fn a_read_only_grant_is_refused_a_write_tool() {
    let resources = create_test_server_resources()
        .await
        .expect("test resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let executor =
        UniversalToolExecutor::new(resources.clone()).with_scopes(vec![OAuthScope::FitnessRead]);

    let error = executor
        .execute_tool(request(
            "set_goal",
            json!({"goal_type": "distance", "target_value": 100.0}),
            user_id,
            tenant,
        ))
        .await
        .expect_err("a read-only grant must not reach a write tool");

    let rendered = error.to_string();
    assert!(
        rendered.contains("fitness:write"),
        "the refusal must name the grant the caller needs, got: {rendered}"
    );

    // The refusal is an authorization failure, not a malformed request: it
    // travels as the structured variant naming the tool, and maps to the 403
    // lane. `InvalidParameters` would answer 400 and tell the caller to fix a
    // request that is fine.
    assert!(
        matches!(
            &error,
            ProtocolError::PermissionDenied { tool_name, reason }
                if tool_name == "set_goal" && reason == "the 'fitness:write' scope is required"
        ),
        "expected PermissionDenied naming the tool and the scope, got: {error:?}"
    );
    let app_error = AppError::from(error);
    assert_eq!(app_error.code, ErrorCode::PermissionDenied);
    assert_eq!(app_error.http_status(), 403);
    // The sentence reaches the client because PermissionDenied is a reviewed
    // passthrough code, not because the code happens to be on a list.
    assert_eq!(
        app_error.sanitized_message(),
        "Permission denied for 'set_goal': the 'fitness:write' scope is required"
    );
}

/// The chat path never reaches an HTTP boundary: a refusal is folded into a
/// failed tool result the model reads inside a 200 turn. That result must
/// carry the refusal's own sentence, and the tool must not be recorded as
/// having run.
#[tokio::test]
async fn a_scope_refusal_on_the_chat_path_is_a_failed_tool_result_naming_the_scope() {
    let resources = create_test_server_resources()
        .await
        .expect("test resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let executor =
        UniversalToolExecutor::new(resources.clone()).with_scopes(vec![OAuthScope::FitnessRead]);
    let calls = vec![FunctionCall {
        name: "set_goal".to_owned(),
        args: json!({"goal_type": "distance", "target_value": 100.0}),
    }];

    let outcome = execute_function_calls(&executor, &calls, &user_id.to_string(), tenant)
        .await
        .expect("dispatch returns an outcome even when a tool is refused");

    assert_eq!(outcome.responses.len(), 1, "the model is owed one response");
    assert_eq!(
        outcome.responses[0].response["error"],
        json!(
            "Tool execution failed: Permission denied for 'set_goal': \
             the 'fitness:write' scope is required"
        ),
        "the failed tool result must carry the refusal verbatim"
    );
    assert!(
        outcome.executed.is_empty(),
        "a refused tool never ran and must not be evidence that it did, got {:?}",
        outcome.executed
    );
}

/// The same tool, with the grant it asks for, runs.
///
/// Paired deliberately with the refusal above: a gate wired to refuse
/// unconditionally passes every negative test in this file, and this is the one
/// that catches it.
#[tokio::test]
async fn the_matching_grant_reaches_the_same_tool() {
    let resources = create_test_server_resources()
        .await
        .expect("test resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let executor =
        UniversalToolExecutor::new(resources.clone()).with_scopes(OAuthScope::self_grant());

    let outcome = executor
        .execute_tool(request(
            "set_goal",
            json!({"goal_type": "distance", "target_value": 100.0}),
            user_id,
            tenant,
        ))
        .await;

    // The tool may still fail for its own reasons in a bare fixture; what must
    // NOT happen is a scope refusal. Asserting on the absence of the scope
    // message rather than on success keeps this test about the gate.
    if let Err(error) = outcome {
        let rendered = error.to_string();
        assert!(
            !rendered.contains("scope"),
            "a sufficient grant must not be refused for scope, got: {rendered}"
        );
    }
}

/// An executor built without a grant refuses, rather than defaulting to
/// everything.
///
/// This is the property that makes a forgotten `with_scopes` fail loudly at the
/// first tool call instead of silently serving a third party the whole
/// registry — the direction a security default has to fail in.
#[tokio::test]
async fn an_unbound_executor_grants_nothing() {
    let resources = create_test_server_resources()
        .await
        .expect("test resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let executor = UniversalToolExecutor::new(resources.clone());

    let error = executor
        .execute_tool(request(
            "set_goal",
            json!({"goal_type": "distance", "target_value": 100.0}),
            user_id,
            tenant,
        ))
        .await
        .expect_err("an executor with no grant must refuse a write tool");

    assert!(
        error.to_string().contains("fitness:write"),
        "the empty grant must refuse and name what was needed, got: {error}"
    );
}
