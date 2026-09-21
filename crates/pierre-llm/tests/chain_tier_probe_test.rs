// ABOUTME: A dead tier behind a live one must be named by the per-tier probe, not hidden by the chain
// ABOUTME: Builds real chains of scripted runners and asserts each tier is asked alone, in order
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A call through a chain exercises only the first tier that answers. On
//! 2026-09-21 `claude_code` answered "Not logged in" on 17 of 17 turns while its
//! version check passed, and nothing said the tier was dead until it was the
//! only one left; the inverse also holds — while the primary answers, a dead
//! fallback is never asked. `probe_chain_tiers` asks each tier on its own.
//!
//! No tier here fails *through* the chain, so the process-global chain guard is
//! never touched and the tests cannot reroute one another.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_llm::health::TierProbe;
use pierre_llm::{ChatMessage, ChatProvider, EmbacleProvider};

/// What a scripted tier does when asked.
enum Behaviour {
    Answers(&'static str),
    Fails(&'static str),
}

struct Scripted {
    label: &'static str,
    behaviour: Behaviour,
    calls: Arc<AtomicUsize>,
    models: Vec<String>,
}

impl Scripted {
    fn tier(label: &'static str, behaviour: Behaviour) -> (EmbacleProvider, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = Self {
            label,
            behaviour,
            calls: Arc::clone(&calls),
            models: vec![format!("{label}-model")],
        };
        (EmbacleProvider::from_runner(Box::new(runner), label), calls)
    }
}

#[async_trait::async_trait]
impl EmbacleLlmProvider for Scripted {
    fn name(&self) -> &'static str {
        self.label
    }

    fn display_name(&self) -> &str {
        self.label
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behaviour {
            Behaviour::Answers(text) => Ok(ChatResponse {
                content: text.to_owned(),
                model: self.models[0].clone(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            }),
            Behaviour::Fails(reason) => Err(RunnerError::internal(reason)),
        }
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("not streamed in this test"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        // The version-probe shape: the binary is there, so this says healthy
        // whatever a completion would do.
        Ok(true)
    }
}

fn ping() -> ChatRequest {
    ChatRequest::new(vec![ChatMessage::user("ping")])
}

#[tokio::test]
async fn a_dead_tier_behind_a_live_one_is_reported() {
    let (primary, primary_calls) = Scripted::tier("primary", Behaviour::Answers("pong"));
    let (secondary, secondary_calls) =
        Scripted::tier("secondary", Behaviour::Fails("Not logged in"));
    let (tertiary, tertiary_calls) = Scripted::tier("tertiary", Behaviour::Answers("pong"));
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![primary, secondary, tertiary]).expect("three tiers"),
    );

    // Through the chain the dead tier is invisible: the primary answers, the
    // version-shaped health check passes, and the secondary is never asked.
    assert!(chain.health_check().await.expect("health check runs"));
    let through_chain = chain.complete(&ping()).await.expect("the primary answers");
    assert_eq!(through_chain.model, "primary-model");
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 0);

    let probes = chain.probe_chain_tiers(&ping()).await;

    assert_eq!(
        probes.len(),
        3,
        "every tier is asked, not only the first to answer"
    );
    assert_eq!(
        probes[0],
        TierProbe {
            provider: "primary",
            position: 0,
            outcome: Ok("primary-model".to_owned()),
        }
    );
    assert_eq!(probes[1].provider, "secondary");
    assert_eq!(probes[1].position, 1);
    let error = probes[1]
        .outcome
        .as_ref()
        .expect_err("the dead tier is reported");
    assert!(
        error.contains("Not logged in"),
        "the tier's own reason must survive, got: {error}"
    );
    assert_eq!(
        probes[2],
        TierProbe {
            provider: "tertiary",
            position: 2,
            outcome: Ok("tertiary-model".to_owned()),
        },
        "a dead tier must not stop the tiers behind it from being asked"
    );

    // Each tier was asked alone: exactly once by the probe, and the dead
    // tier's failure did not spill into a call on its neighbours.
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        2,
        "once through the chain, once alone"
    );
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(tertiary_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_tier_that_answers_with_nothing_is_not_healthy() {
    let (primary, _) = Scripted::tier("primary", Behaviour::Answers("pong"));
    let (secondary, _) = Scripted::tier("secondary", Behaviour::Answers("   "));
    let chain =
        ChatProvider::Embacle(EmbacleProvider::chain(vec![primary, secondary]).expect("two tiers"));

    let probes = chain.probe_chain_tiers(&ping()).await;

    assert_eq!(
        probes[1].outcome,
        Err("empty completion".to_owned()),
        "the chain passes an empty completion over, so the probe must not call it healthy"
    );
}

#[tokio::test]
async fn a_solo_provider_has_no_tiers_to_probe() {
    let (solo, calls) = Scripted::tier("solo", Behaviour::Answers("pong"));
    let provider = ChatProvider::Embacle(solo);

    assert!(provider.probe_chain_tiers(&ping()).await.is_empty());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the chain-level probe already exercises a solo provider; asking again is a wasted completion"
    );
}
