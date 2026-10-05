// ABOUTME: Verifies the strict persona rewrite is re-checked against the contract it was asked to meet
// ABOUTME: Own binary: it captures logs, and tracing caches callsite interest across parallel tests

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pierre_chat_pipeline::stages::persona_conformance::{enforce_conformance, ContractViolation};
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

/// Run one strict repair whose editor returns `rewrite`; return the log.
async fn repair_log(rewrite: &str) -> String {
    let sink = LogSink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        .finish();
    let _guard = set_default(subscriber);
    let provider = Arc::new(ChatProvider::Custom(Arc::new(ScriptedEditor::new(rewrite))));
    let out = enforce_conformance(
        Some(&provider),
        &PromptRegistry::new(),
        &strict_registry(),
        CoachingPersona::Casual,
        "Run 5k easy today, and remember to hydrate well afterwards.".to_owned(),
        &[a_violation()],
        "claude-sonnet-5",
    )
    .await;
    assert_eq!(out, rewrite, "the repair must have gone through the editor");
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

    let still_long = "Run five kilometres easy today and keep it conversational the \
                      whole way, then drink plenty of water afterwards and eat a \
                      proper meal within the hour.";
    let residual = repair_log(still_long).await;
    assert!(
        residual.contains("1 of 1 violation(s) remain") && residual.contains("max_words"),
        "a rewrite still over the cap must report the residual rule, got: {residual}"
    );
}
