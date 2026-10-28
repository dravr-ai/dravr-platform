// ABOUTME: Verifies the strict persona rewrite is re-checked, and that only a shipped non-conforming reply pages
// ABOUTME: Captures logs through one global subscriber writing to a per-thread buffer, so a parallel test cannot hide them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::cell::RefCell;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Once};

use async_trait::async_trait;
use pierre_chat_pipeline::stages::persona_conformance::{
    check_reply_conformance, enforce_conformance, ContractViolation, RosterScope, StyleEditor,
};
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::PromptRegistry;
use pierre_core::errors::AppError;
use pierre_core::models::CoachingPersona;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
};
use tracing::subscriber::set_global_default;
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

thread_local! {
    /// What this thread's events wrote; each test reads only its own.
    static CAPTURED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Writes into the calling thread's [`CAPTURED`] buffer.
///
/// One global subscriber is installed once for the whole binary. A
/// per-test `set_default` raced other tests registering and dropping their
/// own subscribers over tracing's global interest and max-level caches, and
/// on CI an event was sometimes filtered out before it reached the test's
/// subscriber (the log came back empty). With one dispatcher that never
/// changes, those caches never change either. `#[tokio::test]` runs each test
/// on its own current-thread runtime, so the events of a repair are written
/// on the test's own thread.
struct ThreadSink;

impl Write for ThreadSink {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        CAPTURED.with(|captured| captured.borrow_mut().extend_from_slice(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for ThreadSink {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        Self
    }
}

/// Install the capturing subscriber once for this binary.
fn capture_logs() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(|| ThreadSink)
            .with_ansi(false)
            .finish();
        set_global_default(subscriber).expect("no other global subscriber in this binary");
    });
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
    capture_logs();
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    let provider = Arc::new(ChatProvider::Custom(Arc::new(ScriptedEditor::new(
        repair.rewrite,
    ))));
    let editor = StyleEditor {
        provider: Some(&provider),
        prompts: &PromptRegistry::new(),
        model: "claude-sonnet-5",
        recorder: None,
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
    let bytes = CAPTURED.with(|captured| captured.borrow().clone());
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
    assert_residual_alert(&residual, "max_words");
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
    assert_residual_alert(&log, "require_athlete_id_prefix");
}

/// Strict mode pages only when the athlete receives a non-conforming reply.
/// Live 2026-10-28: a French `power_athlete` reply's "environ" paged at
/// detection, though the rewrite repaired it 13 s later. The detection is a
/// warning; the alert waits for the outcome.
#[tokio::test]
async fn a_strict_detection_warns_without_paging() {
    capture_logs();
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    let violations = check_reply_conformance(
        &strict_registry(),
        CoachingPersona::Casual,
        "Run five kilometres easy today and keep it conversational the whole way, \
         then drink plenty of water afterwards and eat a proper meal within the hour.",
        None,
        false,
    );
    assert_eq!(violations.len(), 1, "the reply is over the 20-word cap");
    let log = CAPTURED.with(|captured| String::from_utf8(captured.borrow().clone()).unwrap());
    assert!(
        log.contains("WARN") && log.contains("max_words") && !log.contains("ERROR"),
        "a strict detection is repaired next, so it must warn, not page, got: {log}"
    );
}

/// A rewrite that fails ships the original, rule-breaking reply: that pages.
#[tokio::test]
async fn an_empty_rewrite_pages() {
    capture_logs();
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    let provider = Arc::new(ChatProvider::Custom(Arc::new(ScriptedEditor::new(""))));
    let editor = StyleEditor {
        provider: Some(&provider),
        prompts: &PromptRegistry::new(),
        model: "claude-sonnet-5",
        recorder: None,
    };
    let original = "Run 5k easy today, and remember to hydrate well afterwards.";
    let out = enforce_conformance(
        editor,
        &strict_registry(),
        CoachingPersona::Casual,
        original.to_owned(),
        &[a_violation()],
        false,
    )
    .await;
    assert_eq!(out, original, "a failed rewrite fails open");
    let log = CAPTURED.with(|captured| String::from_utf8(captured.borrow().clone()).unwrap());
    assert!(
        log.contains("ERROR") && log.contains("non-conforming reply shipped"),
        "a failed rewrite must page, got: {log}"
    );
}

/// With no provider to re-prompt, the original rule-breaking reply ships: that
/// pages.
#[tokio::test]
async fn a_strict_repair_without_a_provider_pages() {
    capture_logs();
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    let editor = StyleEditor {
        provider: None,
        prompts: &PromptRegistry::new(),
        model: "claude-sonnet-5",
        recorder: None,
    };
    let original = "Run 5k easy today, and remember to hydrate well afterwards.";
    let out = enforce_conformance(
        editor,
        &strict_registry(),
        CoachingPersona::Casual,
        original.to_owned(),
        &[a_violation()],
        false,
    )
    .await;
    assert_eq!(out, original, "no provider fails open");
    let log = CAPTURED.with(|captured| String::from_utf8(captured.borrow().clone()).unwrap());
    assert!(
        log.contains("ERROR")
            && log.contains("no chat provider to re-prompt")
            && log.contains("rules=max_words"),
        "a repair with no provider must page with its rule, got: {log}"
    );
}

/// Tenant isolation is the deliberate exception: under strict mode a reply
/// citing an athlete outside the coach's roster pages on detection, since the
/// attempt is a security event even though redaction repairs it.
#[tokio::test]
async fn a_strict_isolation_detection_pages() {
    capture_logs();
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    let scope = RosterScope::from_athlete_ids(["11111111-2222-3333-4444-555566667a1b"]);
    let violations = check_reply_conformance(
        &strict_coach_registry(),
        CoachingPersona::Coach,
        "Mallory · dead\nDistance: 42 km\nTime: 3h30",
        Some(&scope),
        true,
    );
    assert!(
        violations
            .iter()
            .any(|v| v.rule == "require_tenant_isolation"),
        "the reply cites an athlete outside the roster, got: {violations:?}"
    );
    let log = CAPTURED.with(|captured| String::from_utf8(captured.borrow().clone()).unwrap());
    assert!(
        log.lines()
            .any(|l| l.contains("ERROR") && l.contains("rule=\"require_tenant_isolation\"")),
        "a strict isolation violation must page on detection, got: {log}"
    );
}

/// A clean rewrite repaired the turn: info only, since WARN and ERROR are
/// forwarded to Slack.
fn assert_no_second_alert(log: &str) {
    assert!(
        !log.contains("WARN") && !log.contains("ERROR"),
        "a repaired turn must not alert, got: {log}"
    );
}

/// A rewrite that still breaks `rule` ships a non-conforming reply: the one
/// strict-mode outcome that pages.
fn assert_residual_alert(log: &str, rule: &str) {
    assert!(
        log.contains("ERROR")
            && log.contains(&format!("still breaks output-style rule '{rule}'"))
            && log.contains(&format!("residual_rules={rule}")),
        "a residual must page with its rule, got: {log}"
    );
}
