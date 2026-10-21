// ABOUTME: The LLM chain as a value the server carries in its config after one boot read
// ABOUTME: Pins whether a chain was configured, its head model, and that a serde round trip keeps both
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The server reads `PIERRE_LLM_CHAIN` once into `ServerConfig::llm_chain`
//! and hands that value to every consumer. "Did the operator choose a chain?"
//! and "which model does the head run?" are answered by the value, never by
//! reading the environment again, so these pin that the value carries the
//! answers and keeps them through the config's serde.

use pierre_llm::config::{LlmChain, LlmProviderType};
use serde_json::{json, Value};

#[test]
fn a_parsed_chain_is_configured_and_the_platform_default_is_not() {
    assert!(LlmChain::parse("copilot_sdk=claude-haiku-4.5").is_configured());
    // An operator who set the variable to nothing valid still set it: the
    // boot log names the repair, and the chain counts as theirs.
    assert!(LlmChain::parse("nope").is_configured());
    assert!(!LlmChain::default().is_configured());
}

#[test]
fn the_head_model_is_the_head_entrys_own() {
    let chain = LlmChain::parse("anthropic=claude-haiku-5-5,gemini=gemini-flash-latest");
    assert_eq!(chain.head().provider, LlmProviderType::Anthropic);
    assert_eq!(chain.head_model().as_deref(), Some("claude-haiku-5-5"));
}

#[test]
fn a_configured_chain_serializes_as_the_value_an_operator_writes() {
    let chain = LlmChain::parse("anthropic=claude-sonnet-5,anthropic#2=claude-haiku-5-5,gemini");
    let raw = serde_json::to_value(&chain).unwrap();
    assert_eq!(raw, json!(chain.to_string()));

    let back: LlmChain = serde_json::from_value(raw).unwrap();
    assert_eq!(back.entries(), chain.entries());
    assert!(back.is_configured());
}

#[test]
fn the_platform_default_serializes_as_absent_and_comes_back_unconfigured() {
    let raw = serde_json::to_value(LlmChain::default()).unwrap();
    assert_eq!(raw, Value::Null);

    let back: LlmChain = serde_json::from_value(raw).unwrap();
    assert_eq!(back, LlmChain::default());
    assert!(!back.is_configured());
}
