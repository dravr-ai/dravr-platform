// ABOUTME: The one bounded re-ask that recovers a dravr-viz block the schema refused
// ABOUTME: Fails open in every direction — a repair may add a chart, never cost the prose
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
use pierre_chat_pipeline::stages::structured_output::SchemaTexts;
use pierre_chat_pipeline::stages::viz_blocks::{extract_viz_blocks, repair_refused_blocks};
use pierre_core::errors::AppError;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
};

fn schemas() -> SchemaTexts {
    let mut s = SchemaTexts::new();
    s.insert("dravr-viz".to_owned(), DRAVR_VIZ_SCHEMA.to_owned());
    s
}

fn granted() -> Vec<String> {
    vec!["chart".to_owned(), "table".to_owned()]
}

fn tools_called() -> Vec<String> {
    vec!["get_activities".to_owned()]
}

/// The 2026-08-31 live-incident reply: a comparison encoded as one series per
/// athlete with a single point each. `points` carries `minItems: 2`, so both
/// series are refused and the athlete gets prose with no chart.
fn reply_with_refused_block() -> String {
    format!(
        "Voici en distance cette semaine :\n\n```dravr-viz\n{}\n```\n",
        r#"{"type":"chart","kind":"bar","source_tool":"get_activities","x":{"label":"Athlete","type":"category"},"series":[{"label":"Toi","points":[["Toi",472.0]]},{"label":"Philippe","points":[["Philippe",29.1]]}]}"#
    )
}

/// The same numbers the schema accepts: one series whose points are the
/// categories.
fn corrected_reply() -> String {
    format!(
        "Voici en distance cette semaine :\n\n```dravr-viz\n{}\n```\n",
        r#"{"type":"chart","kind":"bar","source_tool":"get_activities","x":{"label":"Athlete","type":"category"},"series":[{"label":"Distance","points":[["Toi",472.0],["Philippe",29.1]]}]}"#
    )
}

/// Returns a fixed reply and records what it was asked, so the prompt can be
/// asserted rather than assumed.
struct ScriptedRepairer {
    reply: String,
    seen: Arc<Mutex<Vec<ChatRequest>>>,
    models: Vec<String>,
}

impl ScriptedRepairer {
    /// Not `new`: this hands back the wrapped provider plus the request log,
    /// not a bare `Self`, and naming it `new` trips `wrong_self_convention`.
    fn wired(reply: &str) -> (Arc<ChatProvider>, Arc<Mutex<Vec<ChatRequest>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(ChatProvider::Custom(Arc::new(Self {
            reply: reply.to_owned(),
            seen: Arc::clone(&seen),
            models: vec!["scripted-repairer".to_owned()],
        })));
        (provider, seen)
    }
}

#[async_trait]
impl LlmProvider for ScriptedRepairer {
    fn name(&self) -> &'static str {
        "scripted-repairer"
    }
    fn display_name(&self) -> &'static str {
        "Scripted Repairer"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "scripted-repairer"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.seen
            .lock()
            .expect("mutex is not poisoned")
            .push(request.clone());
        Ok(ChatResponse {
            content: self.reply.clone(),
            model: "scripted-repairer".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal(
            "ScriptedRepairer does not stream; the repair uses complete()",
        ))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A provider that always fails, standing in for an unreachable model.
struct FailingRepairer {
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for FailingRepairer {
    fn name(&self) -> &'static str {
        "failing-repairer"
    }
    fn display_name(&self) -> &'static str {
        "Failing Repairer"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "failing-repairer"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Err(AppError::internal("provider is down"))
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal("provider is down"))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(false)
    }
}

/// End to end: the refused reply goes in, the repaired reply comes back, and
/// extracting it yields the chart the athlete asked for.
///
/// Asserts the recovered block rather than that a string was returned — a
/// repair that returns prose with no valid block is the failure this exists to
/// prevent, and it would pass a weaker assertion.
#[tokio::test]
async fn a_repair_recovers_the_chart_the_schema_refused() {
    let (provider, seen) = ScriptedRepairer::wired(&corrected_reply());

    let first = extract_viz_blocks(
        &schemas(),
        &granted(),
        &tools_called(),
        &reply_with_refused_block(),
    )
    .expect("the reply contains a fence");
    assert!(first.blocks.is_empty(), "the bad block must be refused");
    assert_eq!(first.refusals.len(), 1);

    let repaired = repair_refused_blocks(
        &provider,
        &reply_with_refused_block(),
        &first.refusals,
        "claude-sonnet-5",
    )
    .await
    .expect("a scripted repairer returns a reply");

    let second = extract_viz_blocks(&schemas(), &granted(), &tools_called(), &repaired)
        .expect("the repaired reply contains a fence");
    assert_eq!(
        second.blocks.len(),
        1,
        "the repair must yield a block that validates"
    );
    assert!(
        second.refusals.is_empty(),
        "nothing may remain refused: {:?}",
        second.refusals
    );

    // The repair prompt has to carry the fault that names the field, or the
    // model is being asked to fix something it cannot see.
    let requests = seen.lock().expect("mutex is not poisoned");
    let system = requests[0]
        .messages
        .iter()
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        system.contains("series/0/points"),
        "the repair prompt must name the offending field: {system}"
    );
}

/// The turn's model must be pinned. Sending none resolves to the env default,
/// and on the ACP path a subprocess is pinned at spawn — a mismatch discards
/// the warm subprocess and pays a cold spawn on every repair.
#[tokio::test]
async fn the_repair_pins_the_turns_model() {
    let (provider, seen) = ScriptedRepairer::wired(&corrected_reply());
    let faults = vec!["series/0/points: has less than 2 items".to_owned()];

    repair_refused_blocks(
        &provider,
        &reply_with_refused_block(),
        &faults,
        "gpt-5-mini",
    )
    .await
    .expect("a scripted repairer returns a reply");

    let requests = seen.lock().expect("mutex is not poisoned");
    assert_eq!(
        requests[0].model.as_deref(),
        Some("gpt-5-mini"),
        "the repair must run on the same model as the turn"
    );
}

/// No refusals means no re-ask: the common path must not pay a completion.
#[tokio::test]
async fn nothing_refused_makes_no_provider_call() {
    let (provider, seen) = ScriptedRepairer::wired(&corrected_reply());

    let out = repair_refused_blocks(&provider, "just prose", &[], "claude-sonnet-5").await;

    assert!(out.is_none(), "an empty fault list must not re-ask");
    assert!(
        seen.lock().expect("mutex is not poisoned").is_empty(),
        "the provider must not be called at all"
    );
}

/// A provider error leaves the caller with what it had. The athlete keeps the
/// prose; the repair can only ever add a chart.
#[tokio::test]
async fn a_failing_provider_fails_open() {
    let provider = Arc::new(ChatProvider::Custom(Arc::new(FailingRepairer {
        models: vec!["failing-repairer".to_owned()],
    })));
    let faults = vec!["series/0/points: has less than 2 items".to_owned()];

    let out = repair_refused_blocks(
        &provider,
        &reply_with_refused_block(),
        &faults,
        "claude-sonnet-5",
    )
    .await;

    assert!(out.is_none(), "a provider error must not produce a reply");
}

/// An empty completion is not a repair. Adopting it would blank the athlete's
/// reply outright — strictly worse than the missing chart it was fixing.
#[tokio::test]
async fn an_empty_completion_fails_open() {
    let (provider, _seen) = ScriptedRepairer::wired("   \n  ");
    let faults = vec!["series/0/points: has less than 2 items".to_owned()];

    let out = repair_refused_blocks(
        &provider,
        &reply_with_refused_block(),
        &faults,
        "claude-sonnet-5",
    )
    .await;

    assert!(out.is_none(), "an empty completion must be rejected");
}
