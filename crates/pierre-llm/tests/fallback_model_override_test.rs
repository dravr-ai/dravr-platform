// ABOUTME: Regression test for the runtime fallback chain's PIERRE_LLM_FALLBACK_PROVIDER_MODEL handling
// ABOUTME: Pins behavior that the secondary's default_model is overridden so non-CLI fallbacks like Gemini don't 404 on the primary's model name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Verifies `ChatProvider::with_model` actually rewrites the model for
//! the variants that embed it in the API URL (Gemini, Groq, Local).
//!
//! ## Why this exists
//!
//! Production deployed `PIERRE_LLM_PROVIDER=copilot_headless` with
//! `PIERRE_LLM_DEFAULT_MODEL=claude-opus-4.7` and a Gemini fallback. The
//! Gemini secondary's `from_env()` read `PIERRE_LLM_DEFAULT_MODEL` and
//! issued requests against
//! `https://generativelanguage.googleapis.com/v1beta/models/claude-opus-4.7:generateContent`
//! which 404s. The fallback chain therefore collapsed whenever the
//! primary was unavailable, transitioning the LLM probe to Unhealthy
//! and paging Slack.
//!
//! The fix routes `PIERRE_LLM_FALLBACK_PROVIDER_MODEL` through
//! `ChatProvider::with_model` after the non-CLI fallback is constructed.
//! This test pins that the override actually mutates the model field.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_llm::config::LlmModelConfig;
use pierre_llm::{
    GeminiProvider, GroqProvider, LlmProvider, OpenAiCompatibleConfig, OpenAiCompatibleProvider,
};

#[test]
fn gemini_with_default_model_replaces_inherited_model() {
    let provider = GeminiProvider::with_config(
        "test-api-key",
        &LlmModelConfig {
            default_model: "claude-opus-4.7".to_owned(),
            fallback_model: "claude-sonnet-4.6".to_owned(),
        },
    );
    assert_eq!(
        provider.default_model(),
        "claude-opus-4.7",
        "baseline: GeminiProvider starts with the inherited Claude model name"
    );

    let provider = provider.with_default_model("gemini-3.1-flash-lite");
    assert_eq!(
        provider.default_model(),
        "gemini-3.1-flash-lite",
        "after override, GeminiProvider must use the fallback-provider-specific model \
         so requests target a model name that exists on Google's API"
    );
}

#[test]
fn groq_with_default_model_replaces_inherited_model() {
    let provider = GroqProvider::new("test-api-key".to_owned());
    let baseline = provider.default_model().to_owned();

    let provider = provider.with_default_model("llama-3.3-70b-versatile");
    assert_eq!(
        provider.default_model(),
        "llama-3.3-70b-versatile",
        "after override, GroqProvider must surface the fallback-provider-specific \
         model regardless of what env-driven baseline ({baseline}) was originally used"
    );
}

#[test]
fn local_with_default_model_replaces_inherited_model() {
    let config = OpenAiCompatibleConfig::ollama("qwen2.5:14b-instruct");
    let provider = OpenAiCompatibleProvider::new(config).expect("ollama config builds");
    assert_eq!(
        provider.default_model(),
        "qwen2.5:14b-instruct",
        "baseline: ollama config starts with the seeded model"
    );

    let provider = provider.with_default_model("qwen2.5:32b-instruct");
    assert_eq!(
        provider.default_model(),
        "qwen2.5:32b-instruct",
        "after override, OpenAiCompatibleProvider must use the fallback-provider-specific \
         model — otherwise local-mode fallback would 404 against the primary's model name"
    );
}
