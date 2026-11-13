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

use crate::common;

use common::{create_test_server_resources, create_test_user, create_test_user_with_email};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::FunctionCall;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::function_dispatch::execute_function_calls;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::protocols::ProtocolError;
use pierre_tool_runtime::registry::ToolRegistry;
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
    assert_eq!(
        required_scopes(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
        ),
        []
    );
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
    let (user_id, _) = create_test_user(&resources.agent.database)
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
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let executor =
        UniversalToolExecutor::new(resources.clone()).with_scopes(vec![OAuthScope::FitnessRead]);
    let calls = vec![FunctionCall {
        name: "set_goal".to_owned(),
        args: json!({"goal_type": "distance", "target_value": 100.0}),
    }];

    let outcome = execute_function_calls(&executor, &calls, &user_id.to_string(), tenant, None)
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
    let (user_id, _) = create_test_user(&resources.agent.database)
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
    let (user_id, _) = create_test_user(&resources.agent.database)
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

/// `verify_claim` is offered to, and runs for, a read-only grant.
///
/// The server instructions and every data result's `_reply_verification` tell
/// an external client to call it before stating a figure about the athlete,
/// and the Claude directory connects with only `fitness:read profile:read`.
/// Declared as a write, it would be hidden from that listing and refused at
/// the call (carnet#1510). The listing and the call read the same derivation,
/// so both halves are pinned here: the requirement, and the dispatch.
#[tokio::test]
async fn a_read_only_grant_reaches_verify_claim() {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let caps = registry
        .get("verify_claim")
        .expect("verify_claim is registered")
        .capabilities();
    let read_only = [OAuthScope::FitnessRead, OAuthScope::ProfileRead];
    assert_eq!(
        required_scopes(caps),
        vec![OAuthScope::FitnessRead],
        "verify_claim reads the athlete's record to judge a claim and needs only fitness:read"
    );
    assert_eq!(
        missing_scope(&read_only, caps),
        None,
        "the directory's read grant must cover verify_claim, or tools/list hides it"
    );

    let resources = create_test_server_resources()
        .await
        .expect("test resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let executor = UniversalToolExecutor::new(resources.clone()).with_scopes(read_only.to_vec());

    let outcome = executor
        .execute_tool(request(
            "verify_claim",
            json!({"claim": "I ran 10 km yesterday.", "category": "athlete_data"}),
            user_id,
            tenant,
        ))
        .await
        .expect("a read grant must reach verify_claim, not be refused for scope");

    // It runs, and judges: with nothing connected, no source could have
    // produced the figure, so the athlete's own record contradicts it. A
    // verdict on their data, not the gate waving the call by.
    assert!(
        outcome.success,
        "verify_claim must answer a read grant with a verdict, got: {:?}",
        outcome.error
    );
    let verdict = outcome.result.expect("verify_claim returns its verdict");
    assert_eq!(
        verdict["status"], "contradicted",
        "a figure about an athlete with nothing connected is contradicted, got: {verdict}"
    );
    assert_eq!(
        verdict["layer_fired"], "athlete_data",
        "the athlete's own record decides a figure about them, got: {verdict}"
    );
}

/// `verify_claim` refuses a `conversation_id` the caller does not own.
///
/// A verdict filed under a conversation shows in that conversation's verdict
/// list, so naming someone else's thread would write into what they read. A
/// read grant reaches the tool now, which is why the check sits in the tool
/// itself rather than behind a write scope.
#[tokio::test]
async fn verify_claim_refuses_a_conversation_the_caller_does_not_own() {
    let resources = create_test_server_resources()
        .await
        .expect("test resources");
    let (caller, _) = create_test_user(&resources.agent.database)
        .await
        .expect("caller");
    let (owner, _) =
        create_test_user_with_email(&resources.agent.database, "conversation-owner@example.com")
            .await
            .expect("conversation owner");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let theirs = resources
        .common
        .repos
        .chat
        .create_conversation(&owner.to_string(), tenant, "Theirs", "m", None, None)
        .await
        .expect("the owner's conversation");
    let mine = resources
        .common
        .repos
        .chat
        .create_conversation(&caller.to_string(), tenant, "Mine", "m", None, None)
        .await
        .expect("the caller's conversation");

    let executor = UniversalToolExecutor::new(resources.clone())
        .with_scopes(vec![OAuthScope::FitnessRead, OAuthScope::ProfileRead]);
    let call = |conversation: &str| {
        request(
            "verify_claim",
            json!({
                "claim": "I ran 10 km yesterday.",
                "category": "athlete_data",
                "conversation_id": conversation,
            }),
            caller,
            tenant,
        )
    };

    let refused = executor.execute_tool(call(&theirs.id)).await;
    let rendered = match refused {
        Err(error) => error.to_string(),
        Ok(response) => {
            assert!(
                !response.success,
                "another user's conversation must be refused, got a verdict: {:?}",
                response.result
            );
            response.error.unwrap_or_default()
        }
    };
    assert!(
        rendered.contains("not one of your conversations"),
        "the refusal must say why, got: {rendered}"
    );

    let own = executor
        .execute_tool(call(&mine.id))
        .await
        .expect("the caller's own conversation is accepted");
    assert!(
        own.success,
        "a verdict filed under the caller's own conversation must be written, got: {:?}",
        own.error
    );
}
