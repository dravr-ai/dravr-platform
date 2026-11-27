// ABOUTME: The platform's judge model for dravr-colleur — its prompts sent through a recorded LLM call
// ABOUTME: Colleur writes the prompts and reads the answers; routing, usage and JSON extraction stay here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim judge model
//!
//! dravr-colleur's LLM layers (the claim judge and the rubric judge) never
//! call a provider themselves: they hand a [`JudgeRequest`] to a
//! [`JudgeModel`]. [`RecordedJudge`] is that model on the platform. It sends
//! the request through [`call_record::complete_recorded`], so every judge call is counted
//! in `llm_usage` under the caller's recorder, then extracts the JSON with
//! [`judge::parse_json_response`], the same tolerance every other judge here gets.
//!
//! A provider failure and an answer that holds no JSON come back apart,
//! because colleur asks once more only for the second.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_colleur::{JudgeCallError, JudgeModel, JudgeRequest};
use serde_json::Value;

use crate::call_record::{self, LlmCallRecorder};
use crate::judge;
use crate::LlmProvider;

/// A [`JudgeModel`] over one platform provider, recording each call.
#[derive(Clone, Copy)]
pub struct RecordedJudge<'llm> {
    provider: &'llm dyn LlmProvider,
    recorder: Option<&'llm Arc<dyn LlmCallRecorder>>,
}

impl fmt::Debug for RecordedJudge<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedJudge")
            .field("provider", &self.provider.name())
            .field("recorded", &self.recorder.is_some())
            .finish()
    }
}

impl<'llm> RecordedJudge<'llm> {
    /// A judge on `provider`; `recorder` receives each call's usage, and
    /// `None` leaves the calls unrecorded (benchmarks and tests).
    #[must_use]
    pub fn new(
        provider: &'llm dyn LlmProvider,
        recorder: Option<&'llm Arc<dyn LlmCallRecorder>>,
    ) -> Self {
        Self { provider, recorder }
    }
}

#[async_trait]
impl JudgeModel for RecordedJudge<'_> {
    async fn ask_json(&self, request: JudgeRequest<'_>) -> Result<Value, JudgeCallError> {
        let mut chat = judge::judge_request(
            request.system_prompt,
            request.user_prompt,
            request.temperature,
        );
        if let Some(model) = request.model {
            chat.model = Some(model.to_owned());
        }
        let response = call_record::complete_recorded(self.provider, &chat, self.recorder)
            .await
            .map_err(|e| JudgeCallError::Provider(e.to_string()))?;
        judge::parse_json_response(&response)
            .map_err(|e| JudgeCallError::Unparseable(e.to_string()))
    }
}
