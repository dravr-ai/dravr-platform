// ABOUTME: tools_called must record what RAN, not what the model asked for
// ABOUTME: A failed or hallucinated call must never become evidence a tool produced data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! `execute_function_calls` used to append every name the model *requested* to
//! the turn's `tools_called`, after running them and ignoring every response.
//! A Guardian denial, a tenant-disabled refusal, an errored dispatch and a
//! hallucinated name that matches no registered tool all landed there
//! identically to a success.
//!
//! That matters because `tools_called` is the evidence set for the
//! anti-fabrication gate on coach visuals: a chart carrying
//! `source_tool: "get_activities"` renders only if that tool ran. With the old
//! behaviour a merely-*attempted* call satisfied the citation, so a fabricated
//! chart could be published with provenance that reads as verified.

mod common;

use common::{create_test_server_resources, create_test_user};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::FunctionCall;
use pierre_tool_runtime::function_dispatch::execute_function_calls;
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use serde_json::json;
use uuid::Uuid;

/// A name the coach invented. It matches no registered tool, so dispatch fails
/// — and it must not appear in the executed set.
const HALLUCINATED: &str = "analyze_my_vibes";

#[tokio::test]
async fn a_call_that_did_not_run_is_not_recorded_as_having_run() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    let executor =
        UniversalToolExecutor::new(resources.clone()).with_scopes(OAuthScope::self_grant());

    let calls = vec![
        FunctionCall {
            name: HALLUCINATED.to_owned(),
            args: json!({}),
        },
        FunctionCall {
            name: "get_connection_status".to_owned(),
            args: json!({}),
        },
    ];

    let outcome = execute_function_calls(&executor, &calls, &user_id.to_string(), tenant_id)
        .await
        .expect("dispatch returns an outcome even when a tool fails");

    // The model asked for two; the LLM still gets a response for each so it can
    // see its own mistake.
    assert_eq!(
        outcome.responses.len(),
        2,
        "every requested call still owes the model a response"
    );

    // But only what actually ran is evidence.
    assert!(
        !outcome.executed.iter().any(|t| t == HALLUCINATED),
        "a tool that matches no registry entry must never be recorded as executed, \
         got {:?} — a chart citing it would render with a citation that reads as \
         verified",
        outcome.executed
    );
    assert!(
        outcome.executed.len() < calls.len(),
        "the executed set must be strictly smaller than the requested set when a \
         call failed, got {:?}",
        outcome.executed
    );
}
