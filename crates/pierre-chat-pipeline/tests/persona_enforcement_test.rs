// ABOUTME: Verifies persona conformance enforcement fails open — it never blanks
// ABOUTME: or rewrites a reply unless a strict contract and a provider are present.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pierre_chat_pipeline::stages::persona_conformance::{enforce_conformance, ContractViolation};
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::errors::AppError;
use pierre_core::models::CoachingPersona;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
};

fn a_violation() -> ContractViolation {
    ContractViolation {
        rule: "max_words",
        detail: "reply exceeds the persona word budget".to_owned(),
    }
}

#[tokio::test]
async fn no_violations_returns_reply_unchanged() {
    let registry = Arc::new(PersonaContractRegistry::new());
    let original = "Run 5k easy today.".to_owned();
    let out = enforce_conformance(
        None,
        &registry,
        CoachingPersona::Coach,
        original.clone(),
        &[],
        "claude-sonnet-5",
    )
    .await;
    assert_eq!(out, original);
}

#[tokio::test]
async fn non_strict_contract_is_shadow_mode_only() {
    // A fresh registry has no hydrated (strict) contract, so even with
    // violations present the reply is returned unchanged — shadow mode.
    let registry = Arc::new(PersonaContractRegistry::new());
    let original = "Run 5k easy today, and remember to hydrate well.".to_owned();
    let violations = vec![a_violation()];
    let out = enforce_conformance(
        None,
        &registry,
        CoachingPersona::Coach,
        original.clone(),
        &violations,
        "claude-sonnet-5",
    )
    .await;
    assert_eq!(
        out, original,
        "without a strict contract, enforcement must not alter the reply"
    );
}

/// The rewrite the scripted editor returns for the strict-mode tests.
const REWRITTEN: &str = "Easy 5k today. Drink water.";

/// Minimal provider standing in for the style editor: it returns a fixed
/// rewrite and records the system prompt it was asked with.
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
        // The conformance rewrite is a single blocking completion, so this
        // editor is never streamed. Say so rather than fake a stream.
        Err(AppError::internal(
            "ScriptedEditor does not stream; enforce_conformance uses complete()",
        ))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// Records the model the repair request carried, so the pin can be asserted
/// rather than assumed.
struct ModelCapturingEditor {
    seen: Arc<Mutex<Vec<Option<String>>>>,
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for ModelCapturingEditor {
    fn name(&self) -> &'static str {
        "model-capturing-editor"
    }
    fn display_name(&self) -> &'static str {
        "Model Capturing Editor"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "model-capturing-editor"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.seen
            .lock()
            .expect("capture lock")
            .push(request.model.clone());
        Ok(ChatResponse {
            content: REWRITTEN.to_owned(),
            model: "model-capturing-editor".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal("not used"))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// The persona repair runs on the SAME model as the turn it is repairing.
///
/// Not a style point. On the ACP path a subprocess is pinned to one model at
/// spawn, so a repair that sends no model resolves to the env default; when
/// that differs from the turn's model the pool discards the warm subprocess and
/// pays a ~3.2s cold spawn on EVERY repair turn, silently undoing the pooling.
/// A repair that stopped pinning would still rewrite correctly and still pass
/// every other test here — only this one notices.
#[tokio::test]
async fn the_repair_runs_on_the_same_model_as_the_turn() {
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ChatProvider::Custom(Arc::new(ModelCapturingEditor {
        seen: Arc::clone(&seen),
        models: vec!["model-capturing-editor".to_owned()],
    })));

    let out = enforce_conformance(
        Some(&provider),
        &strict_registry(),
        CoachingPersona::Casual,
        "Run 5k easy today, and remember to hydrate well afterwards.".to_owned(),
        &[a_violation()],
        "claude-opus-4.8",
    )
    .await;
    assert_eq!(
        out, REWRITTEN,
        "the repair must have gone through the editor"
    );

    let captured = seen.lock().expect("capture lock").clone();
    assert_eq!(
        captured,
        vec![Some("claude-opus-4.8".to_owned())],
        "the repair request must carry the turn's model; a None here means it \
         would resolve to the env default and discard the warm subprocess"
    );
}

/// A registry holding one strict contract for `casual`.
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

/// The strict branch is real, not a louder log line.
///
/// Regression guard for a false `LIMITATION` marker: the module doc claimed the
/// re-prompt-with-fix-delta recovery was unbuilt, so `strict_mode: true` was
/// believed to be nothing but `error!` instead of `warn!`. It has in fact been
/// wired since the 2026-06 due-diligence remediation. This pins the behaviour so
/// the claim can be checked instead of believed.
#[tokio::test]
async fn strict_contract_rewrites_the_reply_through_the_editor() {
    let registry = strict_registry();
    let provider = Arc::new(ChatProvider::Custom(Arc::new(ScriptedEditor::new(
        REWRITTEN,
    ))));
    let original = "Run 5k easy today, and remember to hydrate well afterwards.".to_owned();

    let out = enforce_conformance(
        Some(&provider),
        &registry,
        CoachingPersona::Casual,
        original.clone(),
        &[a_violation()],
        "claude-sonnet-5",
    )
    .await;

    assert_eq!(
        out, REWRITTEN,
        "a strict contract with violations must return the editor's rewrite"
    );
    assert_ne!(out, original, "the reply must not pass through untouched");
}

/// Strict mode still fails open: no provider means the reply ships as written,
/// because a style miss must never blank a user's answer.
#[tokio::test]
async fn strict_contract_without_a_provider_keeps_the_original() {
    let registry = strict_registry();
    let original = "Run 5k easy today, and remember to hydrate well afterwards.".to_owned();

    let out = enforce_conformance(
        None,
        &registry,
        CoachingPersona::Casual,
        original.clone(),
        &[a_violation()],
        "claude-sonnet-5",
    )
    .await;

    assert_eq!(out, original);
}
