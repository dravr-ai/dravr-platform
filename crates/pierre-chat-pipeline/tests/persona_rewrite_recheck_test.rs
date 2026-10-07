// ABOUTME: Verifies the strict persona rewrite is re-checked against the contract it was asked to meet
// ABOUTME: Own binary: it captures logs, and tracing caches callsite interest across parallel tests

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pierre_chat_pipeline::stages::persona_conformance::{
    enforce_conformance, ContractViolation, StyleEditor,
};
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::PromptRegistry;
use pierre_core::errors::AppError;
use pierre_core::models::CoachingPersona;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

/// A rewrite inside the 20-word cap of [`strict_registry`].
const REWRITTEN: &str = "Easy 5k today. Drink water.";

fn strict_registry() -> Arc<PersonaContractRegistry> {
    let registry = Arc::new(PersonaContractRegistry::new());
    registry
        .apply_overlay(
            r"
version: 2
personas:
  casual:
    max_words: 20
    strict_mode: true
",
        )
        .expect("overlay applies");
    registry
}

/// Style editor that answers every repair with a fixed rewrite.
struct ScriptedEditor {
    reply: String,
    models: Vec<String>,
}

impl ScriptedEditor {
    fn new(reply: &str) -> Self {
        Self {
            reply: reply.to_owned(),
            models: vec!["scripted-editor".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedEditor {
    fn name(&self) -> &'static str {
        "scripted-editor"
    }
    fn display_name(&self) -> &'static str {
        "Scripted Editor"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "scripted-editor"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: self.reply.clone(),
            model: "scripted-editor".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal(
            "ScriptedEditor does not stream; enforce_conformance uses complete()",
        ))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

fn a_violation() -> ContractViolation {
    ContractViolation {
        rule: "max_words",
        detail: "reply exceeds the persona word budget".to_owned(),
    }
}

/// In-memory sink for the log lines one test emits.
#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0.lock().expect("sink lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogSink {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// One strict repair: the contract, the reply and what it violated, the
/// editor's scripted rewrite, and whether the turn read a coached athlete's
/// data.
struct Repair<'a> {
    registry: Arc<PersonaContractRegistry>,
    persona: CoachingPersona,
    original: &'a str,
    violation: ContractViolation,
    rewrite: &'a str,
    roster_data_read: bool,
}

/// Run one strict repair whose editor returns `rewrite`; return the log.
async fn repair_log(rewrite: &str) -> String {
    run_repair(Repair {
        registry: strict_registry(),
        persona: CoachingPersona::Casual,
        original: "Run 5k easy today, and remember to hydrate well afterwards.",
        violation: a_violation(),
        rewrite,
        roster_data_read: false,
    })
    .await
}

async fn run_repair(repair: Repair<'_>) -> String {
    let sink = LogSink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        .finish();
    let _guard = set_default(subscriber);
    let provider = Arc::new(ChatProvider::Custom(Arc::new(ScriptedEditor::new(
        repair.rewrite,
    ))));
    let editor = StyleEditor {
        provider: Some(&provider),
        prompts: &PromptRegistry::new(),
        model: "claude-sonnet-5",
    };
    let out = enforce_conformance(
        editor,
        &repair.registry,
        repair.persona,
        repair.original.to_owned(),
        &[repair.violation],
        repair.roster_data_read,
    )
    .await;
    assert_eq!(
        out, repair.rewrite,
        "the repair must have gone through the editor"
    );
    let bytes = sink.0.lock().expect("sink lock").clone();
    String::from_utf8(bytes).expect("utf-8 log")
}

/// The rewrite is judged by the same rules it was asked to satisfy
/// (carnet#796): without the re-check the log said only that a rewrite
/// happened, so a re-prompt that fixed nothing looked like one that worked.
#[tokio::test]
async fn the_rewrite_is_rechecked_against_the_contract() {
    let fixed = repair_log(REWRITTEN).await;
    assert!(
        fixed.contains("0 of 1 violation(s) remain"),
        "a rewrite inside the 20-word cap leaves nothing, got: {fixed}"
    );
    assert_no_second_alert(&fixed);

    let still_long = "Run five kilometres easy today and keep it conversational the \
                      whole way, then drink plenty of water afterwards and eat a \
                      proper meal within the hour.";
    let residual = repair_log(still_long).await;
    assert!(
        residual.contains("1 of 1 violation(s) remain")
            && residual.contains("residual_rules=max_words"),
        "a rewrite still over the cap must report the residual rule, got: {residual}"
    );
    assert_no_second_alert(&residual);
}

/// A strict coach, armed through the Power-athlete overlay as in production.
fn strict_coach_registry() -> Arc<PersonaContractRegistry> {
    let registry = Arc::new(PersonaContractRegistry::new());
    registry
        .apply_overlay(
            r"
version: 2
personas:
  power_athlete:
    require_line_by_line_block: true
    strict_mode: true
  coach:
    inherits: power_athlete
    require_athlete_id_prefix: true
    require_tenant_isolation: true
    strict_mode: false
",
        )
        .expect("overlay applies");
    registry
}

/// A coach's own ride report, before and after the editor adds the
/// line-by-line block Power-athlete asks for.
const OWN_RIDE_PROSE: &str = "The Sutton loop is 42 km with 650 m of climbing.";
const OWN_RIDE_BLOCK: &str = "Sutton loop\nDistance: 42 km\nClimbing: 650 m";

fn missing_block() -> ContractViolation {
    ContractViolation {
        rule: "require_line_by_line_block",
        detail: "no label:value block found".to_owned(),
    }
}

/// Live 2026-10-07: a coach asked for rides near home, the editor added the
/// block `require_line_by_line_block` asked for, and the re-check then flagged
/// that block under `require_athlete_id_prefix` — two rules of one contract
/// that no rewrite could satisfy together. The block is the coach's own data
/// on a turn that read no athlete's, and must stand.
#[tokio::test]
async fn a_coachs_own_block_satisfies_the_contract() {
    let log = run_repair(Repair {
        registry: strict_coach_registry(),
        persona: CoachingPersona::Coach,
        original: OWN_RIDE_PROSE,
        violation: missing_block(),
        rewrite: OWN_RIDE_BLOCK,
        roster_data_read: false,
    })
    .await;
    assert!(
        log.contains("0 of 1 violation(s) remain"),
        "the coach's own block needs no athlete citation, got: {log}"
    );
    assert_no_second_alert(&log);
}

/// The same block on a turn that read a roster athlete's data is an athlete
/// report, and still has to say whose it is.
#[tokio::test]
async fn an_uncited_block_after_a_roster_read_remains_a_residual() {
    let log = run_repair(Repair {
        registry: strict_coach_registry(),
        persona: CoachingPersona::Coach,
        original: OWN_RIDE_PROSE,
        violation: missing_block(),
        rewrite: OWN_RIDE_BLOCK,
        roster_data_read: true,
    })
    .await;
    assert!(
        log.contains("1 of 1 violation(s) remain")
            && log.contains("residual_rules=require_athlete_id_prefix"),
        "an athlete report without a citation must be measured, got: {log}"
    );
    assert_no_second_alert(&log);
}

/// WARN and ERROR are forwarded to Slack; the turn's violations already
/// alerted once, so the re-check's measurement must stay below both.
fn assert_no_second_alert(log: &str) {
    assert!(
        !log.contains("WARN") && !log.contains("ERROR"),
        "the re-check must not raise a second alert, got: {log}"
    );
}
