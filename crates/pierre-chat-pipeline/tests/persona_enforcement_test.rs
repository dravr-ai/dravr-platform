// ABOUTME: Verifies persona conformance enforcement fails open — it never blanks
// ABOUTME: or rewrites a reply unless a strict contract and a provider are present.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

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
    )
    .await;

    assert_eq!(out, original);
}
