// ABOUTME: A judge that fails on a claim leaves that claim unchecked, and the agent's fallback decides the reply
// ABOUTME: Drives apply_claim_verification on the real pipeline context against a judge provider that errors

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Claim verification when the LLM judge errors (carnet#851).
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use pierre_chat_pipeline::envelope::VerdictChip;
use pierre_chat_pipeline::stages::verification::{
    apply_claim_verification, persist_pending_verdicts, ClaimVerificationOutcome,
    ClaimVerificationParams,
};
use pierre_contremaitre::messaging_strings::KEY_VERIFICATION_BLOCK_FALLBACK;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
};
use pierre_core::models::TenantId;
use pierre_evals::{EvidenceCorpus, VerificationConfig, VerificationFallback};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::claims::{ClaimStatus, VerdictLayer};
use regex::Regex;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

/// A deterministic bound violation: it never reaches the judge.
const OUT_OF_BOUNDS: &str = "Your maximum heart rate is 300 bpm.";
/// Reaches the judge, because nothing in the corpus can settle it.
const JUDGED_AND_FAILED: &str =
    "Drinking beetroot juice daily raises your lactate threshold by 12% within two weeks.";
/// Reaches the judge too, in another category so it never conflicts with the claim above.
const JUDGED_AND_ANSWERED: &str =
    "Tart cherry juice before bed improves your sleep quality by 8% within a month.";
/// What a claim the judge failed on is recorded as, and what the triage and
/// coach views read back from the verdict row.
const NOT_CHECKED: &str = "Not checked: judge error";

// ── The judge ────────────────────────────────────────────────────────────────

/// A judge provider that times out on one claim and supports every other.
struct FailsOnClaim {
    failing_claim: &'static str,
}

#[async_trait]
impl LlmProvider for FailsOnClaim {
    fn name(&self) -> &'static str {
        "fails_on_claim"
    }
    fn display_name(&self) -> &'static str {
        "Judge that fails on one claim"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let asked_about_failing_claim = request
            .messages
            .iter()
            .any(|m| m.role == MessageRole::User && m.content.contains(self.failing_claim));
        if asked_about_failing_claim {
            return Err(AppError::internal("judge provider timed out"));
        }
        Ok(ChatResponse {
            content:
                r#"{"verdict":"supported","confidence":0.9,"rationale":"Replicated in trials."}"#
                    .to_owned(),
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal("streaming is not used here"))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

// ── Event capture ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: Level,
    message: String,
    fields: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message.clone_from(&rendered);
        }
        self.fields.insert(field.name().to_owned(), rendered);
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

/// The `jsonPayload.message` regex the Cloud Monitoring metric
/// `dravr-claim-verification-failed` filters on.
///
/// This reads the Terraform source on purpose: the log line and the metric
/// filter live on two sides of a boundary no test can cross (the metric runs
/// in Cloud Logging), and a reworded line would otherwise stop paging in
/// silence (docs/coding-standards.md, Tests).
fn metric_filter() -> Regex {
    let terraform =
        include_str!("../../../infra/environments/dev/claim_verification_monitoring.tf");
    let pattern = Regex::new(
        r#"(?s)resource "google_logging_metric" "claim_verification_failed".*?jsonPayload\.message=~"([^"]+)""#,
    )
    .unwrap();
    let captures = pattern.captures(terraform).expect(
        "claim_verification_monitoring.tf must filter the failed metric on jsonPayload.message",
    );
    Regex::new(&captures[1]).unwrap()
}

// ── Fixtures ─────────────────────────────────────────────────────────────────

struct Fixture {
    resources: Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
}

/// A server whose judge provider times out on any claim mentioning `failing_claim`.
async fn setup(failing_claim: &'static str, email: &str) -> Fixture {
    let resources =
        create_test_server_resources_with_chat_provider(Arc::new(FailsOnClaim { failing_claim }))
            .await
            .unwrap();
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    // A corpus holding nothing about beetroot, cherries or heart rates, so the
    // two judged claims are inconclusive for the evidence layer and reach the
    // judge, whatever the compiled-in corpus grows to hold.
    let unrelated = EvidenceCorpus::from_jsonl(
        r#"{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis","strength":"strong","citation":"Morton 2018"}"#,
    )
    .unwrap();
    resources.mcp.evidence_registry.update(
        "nutrition",
        "protein",
        unrelated,
        "sha-unrelated".to_owned(),
    );
    Fixture {
        resources,
        tenant_id,
        user_id,
    }
}

fn config(fallback: VerificationFallback) -> VerificationConfig {
    VerificationConfig {
        fallback_behavior: fallback,
        ..VerificationConfig::default()
    }
}

/// Verify `reply` as the chat pipeline does, for a surface that draws chips.
async fn verify(
    fx: &Fixture,
    reply: &str,
    fallback: VerificationFallback,
) -> ClaimVerificationOutcome {
    let ctx = fx.resources.chat_pipeline_context();
    let config = config(fallback);
    let user_id = fx.user_id.to_string();
    let outcome = apply_claim_verification(ClaimVerificationParams {
        ctx: &ctx,
        renders_chips: true,
        reply,
        config: &config,
        locale: "en",
        user_id: &user_id,
        tenant_id: fx.tenant_id,
        judge_recorder: None,
    })
    .await;
    outcome
}

/// Every event emitted on this thread while the guard lives. The tests run on
/// tokio's current-thread runtime, so every await stays on this thread.
fn capture_events() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let layer = CaptureLayer::default();
    let events = Arc::clone(&layer.events);
    let guard = tracing_subscriber::registry().with(layer).set_default();
    (events, guard)
}

/// The warnings the `dravr-claim-verification-failed` metric counts.
fn counted_by_the_metric(events: &Mutex<Vec<CapturedEvent>>) -> Vec<CapturedEvent> {
    let filter = metric_filter();
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.level == Level::WARN && filter.is_match(&e.message))
        .cloned()
        .collect()
}

// ── The behaviour ────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_blocking_agent_gets_its_block_fallback_when_the_judge_fails() {
    // No claim here is flagged: one is supported by the judge, one is
    // unchecked. Only the judge error can make this agent block.
    let fx = setup("beetroot", "judge-error-block@example.com").await;
    let reply = format!("{JUDGED_AND_ANSWERED} {JUDGED_AND_FAILED}");
    let (events, guard) = capture_events();

    let outcome = verify(&fx, &reply, VerificationFallback::Block).await;
    drop(guard);

    let block_fallback = fx
        .resources
        .mcp
        .messaging_strings_registry
        .get(KEY_VERIFICATION_BLOCK_FALLBACK, "en");
    assert!(!block_fallback.is_empty());
    assert_eq!(
        outcome.content, block_fallback,
        "a blocking coach must not deliver a reply it could not check"
    );
    assert_ne!(outcome.content, reply);
    assert!(
        outcome.chips.is_empty(),
        "a replaced reply carries no chips"
    );

    // The rows survive the block: the triage tab sees what was decided and
    // what was not.
    let statuses: Vec<(ClaimStatus, bool)> = outcome
        .pending_verdicts
        .iter()
        .map(|(_, verdict)| (verdict.status, verdict.is_judge_error()))
        .collect();
    assert_eq!(
        statuses,
        [
            (ClaimStatus::Supported, false),
            (ClaimStatus::Unverifiable, true)
        ]
    );

    let counted = counted_by_the_metric(&events);
    assert_eq!(counted.len(), 1, "one line per reply: {counted:#?}");
    assert_eq!(counted[0].fields["unchecked_claims"], "1");
    assert_eq!(counted[0].fields["blocked"], "true");
}

#[tokio::test]
async fn a_warning_agent_ships_the_reply_and_its_rows_record_the_claim_as_not_checked() {
    let fx = setup("beetroot", "judge-error-warn@example.com").await;
    let reply = format!("{OUT_OF_BOUNDS} {JUDGED_AND_FAILED} {JUDGED_AND_ANSWERED}");
    let (events, guard) = capture_events();

    let outcome = verify(&fx, &reply, VerificationFallback::Warn).await;
    drop(guard);

    assert_eq!(outcome.content, reply, "the reply ships");
    assert_eq!(
        outcome.chips,
        [VerdictChip {
            claim: OUT_OF_BOUNDS.to_owned(),
            contradicted: true,
        }],
        "the deterministic layer still rules, and the unchecked claim earns no chip"
    );

    let counted = counted_by_the_metric(&events);
    assert_eq!(counted.len(), 1, "one line per reply: {counted:#?}");
    assert_eq!(counted[0].fields["unchecked_claims"], "1");
    assert_eq!(counted[0].fields["blocked"], "false");

    // The rows, as the triage tab and the coach view read them back.
    let conversation = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.user_id.to_string(),
            fx.tenant_id,
            "Verdicts",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap();
    persist_pending_verdicts(
        &fx.resources.data(),
        fx.tenant_id,
        &fx.user_id.to_string(),
        &conversation.id,
        None,
        "assistant-message-1",
        &outcome.pending_verdicts,
    )
    .await;
    let rows = fx
        .resources
        .common
        .repos
        .claim_verdicts
        .list_verdicts_for_message(fx.tenant_id, "assistant-message-1")
        .await
        .unwrap();
    let row_for = |claim: &str| {
        rows.iter()
            .find(|row| row.claim_text == claim)
            .unwrap_or_else(|| panic!("no verdict row for {claim:?}: {rows:#?}"))
    };
    assert_eq!(rows.len(), 3, "no claim loses its row to the judge error");

    let bound = row_for(OUT_OF_BOUNDS);
    assert_eq!(bound.status, ClaimStatus::Contradicted);
    assert_eq!(bound.layer_fired, VerdictLayer::Deterministic);

    let unchecked = row_for(JUDGED_AND_FAILED);
    assert_eq!(unchecked.status, ClaimStatus::Unverifiable);
    assert_eq!(unchecked.layer_fired, VerdictLayer::Judge);
    assert_eq!(unchecked.explanation.as_deref(), Some(NOT_CHECKED));

    let answered = row_for(JUDGED_AND_ANSWERED);
    assert_eq!(answered.status, ClaimStatus::Supported);
    assert_eq!(answered.layer_fired, VerdictLayer::Judge);
}

#[tokio::test]
async fn a_healthy_judge_neither_blocks_nor_pages() {
    // The block is for a reply that could not be checked. A blocking agent
    // whose judge answered every claim keeps its reply.
    let fx = setup("a claim no reply contains", "judge-healthy@example.com").await;
    let (events, guard) = capture_events();

    let outcome = verify(&fx, JUDGED_AND_ANSWERED, VerificationFallback::Block).await;
    drop(guard);

    assert_eq!(outcome.content, JUDGED_AND_ANSWERED);
    assert_eq!(outcome.pending_verdicts.len(), 1);
    assert!(!outcome.pending_verdicts[0].1.is_judge_error());
    assert!(counted_by_the_metric(&events).is_empty());
}
