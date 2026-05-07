// ABOUTME: Probe test that calls the real Copilot CLI via embacle and dumps response.usage
// ABOUTME: Gated by PIERRE_PROBE_COPILOT=1; verifies what Copilot ACP actually returns for token usage

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Diagnostic probe that calls the real Copilot CLI via embacle and dumps
//! `response.usage` to stdout. Gated by `PIERRE_PROBE_COPILOT=1` so CI
//! never spawns the subprocess; run locally to verify what the Copilot
//! ACP transport actually returns for token counts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::env;

use embacle::types::LlmProvider as EmbacleLlmProvider;
use pierre_llm::{ChatMessage, ChatRequest, CopilotHeadlessConfig, CopilotHeadlessRunner};

#[tokio::test(flavor = "multi_thread")]
async fn probe_copilot_headless_response_usage() {
    // Gated to avoid spawning a real Copilot subprocess in CI; run locally with:
    //   PIERRE_PROBE_COPILOT=1 cargo test --test copilot_usage_probe_test -- --nocapture
    if env::var("PIERRE_PROBE_COPILOT").is_err() {
        eprintln!("[probe] PIERRE_PROBE_COPILOT not set; skipping");
        return;
    }

    let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::from_env());
    let request = ChatRequest::new(vec![ChatMessage::user(
        "Reply with the single word: pong. No other content.",
    )]);

    let response = EmbacleLlmProvider::complete(&runner, &request)
        .await
        .expect("copilot complete must succeed for the probe");

    println!("===== COPILOT HEADLESS USAGE PROBE =====");
    println!("model: {}", response.model);
    println!("finish_reason: {:?}", response.finish_reason);
    println!("content_len: {}", response.content.len());
    println!("content: {}", response.content);
    println!("usage.is_some(): {}", response.usage.is_some());
    println!("usage: {:?}", response.usage);
    println!("========================================");
}
