// ABOUTME: Generic LLM-as-judge helpers for structured JSON verdicts
// ABOUTME: Shared prompt/response plumbing consumed by insight validation, coaching evals, etc.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Judge Helpers
//!
//! Generic utilities for the "LLM-as-judge" pattern: prompt an LLM with a
//! rubric, receive a structured JSON verdict, parse it into a typed
//! response. Consumed by social feed insight validation today; built to
//! be reused by the coaching evaluation harness and any other caller
//! that needs an LLM to return a structured verdict.
//!
//! The helpers deliberately do not own a `JudgeVerdict` type — callers
//! keep their own domain-shaped verdicts and deserialize directly into
//! them via [`ask_for_json`].

use pierre_core::errors::{AppError, AppResult};
use pierre_core::llm::{ChatMessage, ChatRequest};
use serde::de::DeserializeOwned;
use tracing::warn;

use crate::LlmProvider;

/// Call an LLM with a system prompt + user message and deserialize the
/// response as JSON into a caller-chosen type.
///
/// The response text is passed through [`extract_json`] so it tolerates
/// LLMs that wrap JSON in prose or fenced code blocks.
///
/// # Errors
///
/// Returns an error if the LLM call fails, the response cannot be
/// resolved to valid JSON, or the JSON does not deserialize into `T`.
pub async fn ask_for_json<T: DeserializeOwned>(
    provider: &dyn LlmProvider,
    system_prompt: &str,
    user_message: &str,
    temperature: f32,
) -> AppResult<T> {
    let messages = vec![
        ChatMessage::system(system_prompt.to_owned()),
        ChatMessage::user(user_message.to_owned()),
    ];
    let request = ChatRequest::new(messages).with_temperature(temperature);
    let response = provider.complete(&request).await?;

    let json_str = extract_json(&response.content)?;
    serde_json::from_str::<T>(&json_str).map_err(|e| {
        warn!("Failed to deserialize judge response: {e}");
        AppError::internal(format!("Failed to parse judge response: {e}"))
    })
}

/// Extract a JSON payload from an LLM response that may include prose or
/// fenced code blocks.
///
/// Strategy (in order):
/// 1. Whole response parses as JSON → return verbatim.
/// 2. Substring between first `{` and last `}` parses as JSON → return it.
/// 3. A `` ```json `` fenced block contains JSON → recurse into its body.
///
/// Returns an [`AppError::internal`] if none of these succeed.
///
/// # Errors
///
/// Returns an error when no JSON object can be recovered from the text.
pub fn extract_json(response: &str) -> AppResult<String> {
    if serde_json::from_str::<serde_json::Value>(response).is_ok() {
        return Ok(response.to_owned());
    }

    if let (Some(start), Some(end)) = (response.find('{'), response.rfind('}')) {
        if start <= end {
            let candidate = &response[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_owned());
            }
        }
    }

    if let Some(fence_start) = response.find("```json") {
        let after_fence = &response[fence_start + "```json".len()..];
        if let Some(fence_end) = after_fence.find("```") {
            let body = after_fence[..fence_end].trim();
            return extract_json(body);
        }
    }

    Err(AppError::internal(
        "Could not extract valid JSON from LLM response",
    ))
}

#[cfg(test)]
mod tests {
    use super::extract_json;
    use pierre_core::errors::AppResult;

    #[test]
    fn extracts_whole_json() -> AppResult<()> {
        let input = r#"{"verdict":"valid","reason":"ok"}"#;
        assert_eq!(extract_json(input)?, input);
        Ok(())
    }

    #[test]
    fn extracts_json_surrounded_by_prose() -> AppResult<()> {
        let input = r#"Sure, here is my verdict:
{"verdict":"rejected","reason":"too short"}
Hope that helps!"#;
        let result = extract_json(input)?;
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
        assert!(result.contains("rejected"));
        Ok(())
    }

    #[test]
    fn extracts_fenced_json_block() -> AppResult<()> {
        let input = "Here you go:\n```json\n{\"verdict\":\"valid\",\"reason\":\"ok\"}\n```\nEnjoy.";
        assert!(extract_json(input)?.contains("valid"));
        Ok(())
    }

    #[test]
    fn rejects_response_without_json() {
        let input = "I'm not sure what to say.";
        assert!(extract_json(input).is_err());
    }
}
