// ABOUTME: Generic LLM-as-judge helpers for structured JSON verdicts
// ABOUTME: Shared prompt/response plumbing consumed by the outcome evaluator and coaching evals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Judge Helpers
//!
//! Generic utilities for the "LLM-as-judge" pattern: prompt an LLM with a
//! rubric, receive a structured JSON verdict, parse it into a typed
//! response. Consumed by the outcome evaluator and the coaching evaluation
//! harness, and open to any other caller that needs an LLM to return a
//! structured verdict.
//!
//! The helpers deliberately do not own a `JudgeVerdict` type — callers
//! keep their own domain-shaped verdicts and deserialize directly into
//! them via [`ask_for_json`]. A production judge passes a recorder, so its
//! calls are counted in `llm_usage` apart from the reply they judge.

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::llm::{ChatMessage, ChatRequest, ChatResponse};
use serde::de::DeserializeOwned;
use tracing::warn;

use crate::call_record::{complete_recorded, LlmCallRecorder};
use crate::provider_stop::ProviderStop;
use crate::LlmProvider;

/// The request a judge call sends: the rubric as the system prompt, the case
/// as the user message.
///
/// Built apart from [`ask_for_json`] so a caller can route it — set the model
/// a judge stage runs on — before it is sent.
#[must_use]
pub fn judge_request(system_prompt: &str, user_message: &str, temperature: f32) -> ChatRequest {
    ChatRequest::new(vec![
        ChatMessage::system(system_prompt.to_owned()),
        ChatMessage::user(user_message.to_owned()),
    ])
    .with_temperature(temperature)
}

/// Send a judge `request` and deserialize the response as JSON into a
/// caller-chosen type, handing the call's usage to `recorder`.
///
/// The response goes through [`parse_json_response`], so it tolerates LLMs
/// that wrap JSON in prose or fenced code blocks.
///
/// # Errors
///
/// Returns an error if the LLM call fails, the response cannot be
/// resolved to valid JSON, or the JSON does not deserialize into `T`.
pub async fn ask_for_json<T: DeserializeOwned>(
    provider: &dyn LlmProvider,
    request: &ChatRequest,
    recorder: Option<&Arc<dyn LlmCallRecorder>>,
) -> AppResult<T> {
    let response = complete_recorded(provider, request, recorder).await?;
    parse_json_response(&response)
}

/// Deserialize a judge's completed `response` into `T`, via [`extract_json`].
///
/// Apart from [`ask_for_json`] so a caller can tell a provider failure (the
/// call errored) from an answer that held no usable JSON, and re-ask on the
/// second only.
///
/// # Errors
///
/// Returns an error if no JSON object can be recovered from the content, or
/// it does not deserialize into `T`.
pub fn parse_json_response<T: DeserializeOwned>(response: &ChatResponse) -> AppResult<T> {
    let json_str = extract_json(&response.content).inspect_err(|_| {
        // The text can echo the athlete's reply, so only its shape is logged:
        // enough to tell a cut-off object from prose or a refusal.
        let content = &response.content;
        warn!(
            model = %response.model,
            finish_reason = response.finish_reason.as_deref().unwrap_or("none"),
            stop = ?ProviderStop::from_finish_reason(response.finish_reason.as_deref()),
            content_len = content.len(),
            open_braces = content.matches('{').count(),
            close_braces = content.matches('}').count(),
            fences = content.matches("```").count(),
            "judge response held no parseable JSON object"
        );
    })?;
    serde_json::from_str::<T>(&json_str).map_err(|e| {
        // serde's message can quote a field value, which can echo the
        // athlete's reply; its category and position are enough to triage.
        let shape = format!(
            "{:?} error at line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        );
        warn!(error = %shape, "Failed to deserialize judge response");
        AppError::internal(format!("Failed to parse judge response: {shape}"))
    })
}

/// Extract a JSON payload from an LLM response that may include prose or
/// fenced code blocks.
///
/// Strategy (in order):
/// 1. Whole response parses as JSON → return verbatim.
/// 2. A fenced block (`` ```json `` or a bare `` ``` ``) contains JSON →
///    recurse into its body.
/// 3. The first balanced `{…}` span that parses as JSON → return it. Braces
///    inside JSON strings are not counted, so a rationale quoting `{` or a
///    trailing remark with a stray `}` cannot shift the span.
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

    if let Some(body) = fenced_body(response) {
        if let Ok(json) = extract_json(body) {
            return Ok(json);
        }
    }

    let mut from = 0;
    while let Some(offset) = response[from..].find('{') {
        let start = from + offset;
        if let Some(len) = balanced_object_len(&response[start..]) {
            let candidate = &response[start..start + len];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_owned());
            }
        }
        from = start + 1;
    }

    Err(AppError::internal(
        "Could not extract valid JSON from LLM response",
    ))
}

/// The trimmed body of the first closed `` ``` `` fence, with an optional
/// `json` language tag dropped.
fn fenced_body(response: &str) -> Option<&str> {
    let after_open = &response[response.find("```")? + 3..];
    let after_tag = after_open.strip_prefix("json").unwrap_or(after_open);
    let close = after_tag.find("```")?;
    Some(after_tag[..close].trim())
}

/// Byte length of the brace-balanced span opening at `text[0] == '{'`,
/// skipping braces inside JSON strings; `None` when it never closes.
fn balanced_object_len(text: &str) -> Option<usize> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in text.char_indices() {
        if in_string {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}
