// ABOUTME: A tier that rejects every request, the probe included, is benched and the tiers behind it serve
// ABOUTME: Builds real chains of scripted runners; pins the 2026-11-20 outage and the request that really is wrong
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-11-20 the chain's head reported a spent credit as a rejected
//! request. The chain does not fall through on a rejected request, so every
//! call died on the head for 14 hours with six healthy tiers behind it.
//!
//! The chain cannot tell that error from a request that really is wrong by its
//! kind. It can by asking the tier for the probe completion: a tier that
//! answers "ping" rejected the request, and one that cannot is dead. These
//! tests drive that second opinion through `ChatProvider::complete`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_core::errors::ErrorCode;
use pierre_llm::{ChatMessage, ChatProvider, EmbacleProvider, QuotaCooldowns};
use tokio::sync::Barrier;

use crate::chain_guard_lock::fresh_chain_guard;

/// What Anthropic's Messages API answered for the spent organization.
const SPENT_CREDIT: &str =
    "Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits.";

/// A request the vendor is right to reject.
const MALFORMED: &str = "messages: tool_result block does not match a tool_use id";

/// What a scripted tier does when asked.
#[derive(Clone, Copy)]
enum Behaviour {
    /// Answers every request.
    Answers,
    /// Rejects every request as invalid, the probe included, while `dead`
    /// holds; answers once it is cleared.
    RejectsEverything,
    /// Answers the probe and rejects anything else as invalid.
    RejectsAllButThePing,
    /// Refuses every request on quota: a provider fault the chain falls
    /// through on, starting the tier's cooldown.
    RefusesOnQuota,
    /// Answers the probe and fails anything else with a provider fault the
    /// chain falls through on without starting a cooldown.
    FaultsOnAllButThePing,
    /// Rejects a real request as invalid and fails the probe with a provider
    /// fault: a live tier whose vendor blips at the wrong moment.
    RejectsAndFaultsOnThePing,
    /// Fails every request with a provider fault: a runner that is down in
    /// a way the chain falls through.
    Faults,
}

struct Scripted {
    label: &'static str,
    behaviour: Behaviour,
    dead: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    /// Holds every request but the probe until as many are waiting as the
    /// barrier counts, so calls in flight together get their answers together.
    gate: Option<Arc<Barrier>>,
    models: Vec<String>,
}

/// What a test reads and flips on a scripted tier once the chain owns it.
struct Tier {
    calls: Arc<AtomicUsize>,
    dead: Arc<AtomicBool>,
}

impl Tier {
    fn new(label: &'static str, behaviour: Behaviour) -> (EmbacleProvider, Self) {
        Self::built(label, behaviour, None)
    }

    /// A tier that answers its questions only once `gate` has them all.
    fn gated(
        label: &'static str,
        behaviour: Behaviour,
        gate: Arc<Barrier>,
    ) -> (EmbacleProvider, Self) {
        Self::built(label, behaviour, Some(gate))
    }

    fn built(
        label: &'static str,
        behaviour: Behaviour,
        gate: Option<Arc<Barrier>>,
    ) -> (EmbacleProvider, Self) {
        let calls = Arc::new(AtomicUsize::new(0));
        let dead = Arc::new(AtomicBool::new(true));
        let runner = Scripted {
            label,
            behaviour,
            dead: Arc::clone(&dead),
            calls: Arc::clone(&calls),
            gate,
            models: vec![format!("{label}-model")],
        };
        (
            EmbacleProvider::from_runner(Box::new(runner), label),
            Self { calls, dead },
        )
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Scripted {
    fn answer(&self) -> ChatResponse {
        ChatResponse {
            replay_blocks: None,
            content: "pong".to_owned(),
            model: self.models[0].clone(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        }
    }
}

fn is_the_ping(request: &ChatRequest) -> bool {
    request.messages.len() == 1 && request.messages[0].content == "ping"
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

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = self.gate.as_ref().filter(|_| !is_the_ping(request)) {
            gate.wait().await;
        }
        let rejection = match self.behaviour {
            Behaviour::RefusesOnQuota => {
                return Err(RunnerError::rate_limit(self.label, "usage limit reached"));
            }
            Behaviour::FaultsOnAllButThePing if !is_the_ping(request) => {
                return Err(RunnerError::internal("runner crashed mid-request"));
            }
            Behaviour::Faults => return Err(RunnerError::internal("runner is down")),
            Behaviour::RejectsAndFaultsOnThePing if is_the_ping(request) => {
                return Err(RunnerError::internal("529 overloaded"));
            }
            Behaviour::RejectsAndFaultsOnThePing => Some(MALFORMED),
            Behaviour::RejectsEverything if self.dead.load(Ordering::SeqCst) => Some(SPENT_CREDIT),
            Behaviour::RejectsAllButThePing if !is_the_ping(request) => Some(MALFORMED),
            Behaviour::Answers
            | Behaviour::RejectsEverything
            | Behaviour::RejectsAllButThePing
            | Behaviour::FaultsOnAllButThePing => None,
        };
        rejection.map_or_else(
            || Ok(self.answer()),
            |why| Err(RunnerError::invalid_request(why)),
        )
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("not streamed in this test"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

/// A real turn's request: anything but the probe.
fn question() -> ChatRequest {
    ChatRequest::new(vec![ChatMessage::user("How was my week?")])
}

fn chain_of(tiers: Vec<EmbacleProvider>) -> ChatProvider {
    ChatProvider::Embacle(EmbacleProvider::chain(tiers, QuotaCooldowns::default()).expect("tiers"))
}

/// The outage as it began: an instance that booted healthy, and a head that
/// then rejects every request as invalid. The call is served by the tier
/// behind it, and the head is not asked again.
#[tokio::test]
async fn a_head_that_rejects_everything_is_benched_and_the_next_tier_answers() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (second_tier, second) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier]);

    let served = chain
        .complete(&question())
        .await
        .expect("the tier behind a dead head serves the call");

    assert_eq!(served.model, "copilot_sdk-model");
    assert_eq!(
        head.calls(),
        2,
        "the head is asked the question, then the probe"
    );
    assert!(chain.is_tier_benched("anthropic"));

    chain
        .complete(&question())
        .await
        .expect("the next call is served too");
    assert_eq!(
        head.calls(),
        2,
        "a benched head is passed over, not asked again"
    );
    assert_eq!(second.calls(), 2);
}

/// A request that really is wrong fails as it always did: the head answers
/// the probe, so the rejection stands and no other tier is spent on it.
#[tokio::test]
async fn a_rejected_request_from_a_live_head_stays_rejected() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("anthropic", Behaviour::RejectsAllButThePing);
    let (second_tier, second) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier]);

    let error = chain
        .complete(&question())
        .await
        .expect_err("a malformed request is not served by another tier");

    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains(MALFORMED), "{}", error.message);
    assert_eq!(
        head.calls(),
        2,
        "the second opinion costs one probe on the tier that rejected"
    );
    assert_eq!(second.calls(), 0, "no other provider's quota is spent");
    assert!(!chain.is_tier_benched("anthropic"));
}

/// The outage as it continued: every new instance boots with the head already
/// dead. The startup probe benches it, so the first turn never meets it.
#[tokio::test]
async fn a_tier_benched_at_boot_is_never_asked_by_a_turn() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (second_tier, second) = Tier::new("anthropic#2", Behaviour::Answers);
    let (third_tier, third) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier, third_tier]);

    let probes = chain.probe_and_bench_chain_tiers().await;
    assert!(probes[0].outcome.is_err(), "the head failed its probe");
    assert_eq!(head.calls(), 1, "the startup probe asked the head once");

    let served = chain.complete(&question()).await.expect("served");

    assert_eq!(served.model, "anthropic#2-model");
    assert_eq!(head.calls(), 1, "the turn passed the benched head over");
    assert_eq!(second.calls(), 2, "its own probe, then the turn");
    assert_eq!(third.calls(), 1, "its own probe only");
}

/// Two dead tiers in a row: each is found by its own probe, and the third
/// tier serves the same call.
#[tokio::test]
async fn every_dead_tier_ahead_of_a_live_one_is_benched_in_one_call() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, _) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (second_tier, _) = Tier::new("anthropic#2", Behaviour::RejectsEverything);
    let (third_tier, _) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier, third_tier]);

    let served = chain.complete(&question()).await.expect("served");

    assert_eq!(served.model, "copilot_sdk-model");
    assert!(chain.is_tier_benched("anthropic"));
    assert!(chain.is_tier_benched("anthropic#2"));
}

/// Two calls in flight when the head dies are both rejected by it. The first
/// to hear so benches the head; the other finds the tier it asked already
/// passed over and is made again on the tier behind, without a probe of a
/// tier that rejected nothing.
#[tokio::test]
async fn two_calls_rejected_together_by_a_dead_head_are_both_served() {
    let _chain_guard = fresh_chain_guard().await;
    let both_asked = Arc::new(Barrier::new(2));
    let (head_tier, head) = Tier::gated("anthropic", Behaviour::RejectsEverything, both_asked);
    let (second_tier, second) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier]);
    let (one, other) = (question(), question());

    let (first, second_call) = tokio::join!(chain.complete(&one), chain.complete(&other));

    assert_eq!(
        first.expect("served").model,
        "copilot_sdk-model",
        "the tier behind the dead head serves the first call"
    );
    assert_eq!(
        second_call.expect("served").model,
        "copilot_sdk-model",
        "and the call that was in flight beside it"
    );
    assert_eq!(head.calls(), 3, "the two questions, then one probe");
    assert_eq!(
        second.calls(),
        2,
        "the live tier answers the two questions and is never probed"
    );
}

/// A call that falls through a head refusing on quota and is then rejected by
/// a live tier fails as it is: the probe goes to the tier that rejected, which
/// answers it, and the wrong request is not sent to that tier a second time.
#[tokio::test]
async fn a_request_rejected_behind_a_quota_refusal_is_not_sent_again() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("anthropic", Behaviour::RefusesOnQuota);
    let (second_tier, second) = Tier::new("anthropic#2", Behaviour::RejectsAllButThePing);
    let (third_tier, third) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier, third_tier]);

    let error = chain
        .complete(&question())
        .await
        .expect_err("a malformed request is not served by another tier");

    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(head.calls(), 1, "the head refused once and is cooling");
    assert_eq!(
        second.calls(),
        2,
        "the tier that rejected is asked the question once, then the probe"
    );
    assert_eq!(third.calls(), 0, "no other provider's quota is spent");
    assert!(!chain.is_tier_benched("anthropic#2"));
}

/// The probe goes to the tier that rejected, not to the first tier of the
/// chain: a head that faulted on the question and would answer "ping" proves
/// nothing about the dead tier behind it.
#[tokio::test]
async fn a_dead_tier_behind_a_faulting_head_is_the_one_probed() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("copilot_sdk", Behaviour::FaultsOnAllButThePing);
    let (second_tier, second) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (third_tier, third) = Tier::new("gemini", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier, third_tier]);

    let served = chain
        .complete(&question())
        .await
        .expect("the tier behind the dead one serves the call");

    assert_eq!(served.model, "gemini-model");
    assert!(
        chain.is_tier_benched("anthropic"),
        "the tier that rejected is benched"
    );
    assert!(
        !chain.is_tier_benched("copilot_sdk"),
        "a head that faulted is not probed and not benched"
    );
    assert_eq!(
        head.calls(),
        1,
        "the question once: the call resumes behind the dead tier, not at the head"
    );
    assert_eq!(second.calls(), 2, "the question, then the probe");
    assert_eq!(third.calls(), 1);
}

/// A router tier is named after its active backend, so two tiers of one
/// chain can report the same name. The tier that rejected is told apart by
/// its position: a dead account that sits behind itself under one name ends
/// the call with its error instead of being asked for ever.
#[tokio::test]
async fn two_tiers_under_one_name_end_the_call_instead_of_repeating_it() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("claude_code", Behaviour::RejectsEverything);
    let (router_tier, router) = Tier::new("claude_code", Behaviour::RejectsEverything);
    let chain = chain_of(vec![head_tier, router_tier]);

    let error = chain
        .complete(&question())
        .await
        .expect_err("nothing answers");

    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(head.calls(), 2, "the question, then the probe");
    assert_eq!(
        router.calls(),
        1,
        "the last tier is asked once; its rejection is not read as the head's"
    );
}

/// One failed probe is not a death sentence. A tier that rejects a request
/// and whose probe then fails with a provider fault is not benched: the fault
/// says nothing about the request, so the rejection stands and no other
/// provider's quota is spent on it.
#[tokio::test]
async fn a_probe_that_faults_benches_nothing_and_the_rejection_stands() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("anthropic", Behaviour::RejectsAndFaultsOnThePing);
    let (second_tier, second) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier]);

    let error = chain
        .complete(&question())
        .await
        .expect_err("the rejection stands");

    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains(MALFORMED), "{}", error.message);
    assert!(!chain.is_tier_benched("anthropic"));
    assert_eq!(head.calls(), 2, "the question, then the probe");
    assert_eq!(
        second.calls(),
        0,
        "the next tier is not sent a wrong request"
    );
}

/// At startup, a tier that fails its probe with a provider fault is reported
/// and left in the chain, which falls through it on that same fault; only a
/// tier that rejects the probe is benched.
#[tokio::test]
async fn a_tier_that_faults_at_boot_is_reported_but_not_benched() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, _) = Tier::new("copilot_sdk", Behaviour::Faults);
    let (second_tier, _) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (third_tier, _) = Tier::new("gemini", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier, third_tier]);

    let probes = chain.probe_and_bench_chain_tiers().await;

    assert!(probes[0].outcome.is_err(), "the faulting head is reported");
    assert!(probes[1].outcome.is_err(), "the dead tier is reported");
    assert!(
        !chain.is_tier_benched("copilot_sdk"),
        "a provider fault is not a bench"
    );
    assert!(chain.is_tier_benched("anthropic"));

    let served = chain.complete(&question()).await.expect("served");
    assert_eq!(
        served.model, "gemini-model",
        "the chain falls through the faulting head and passes the benched tier over"
    );
}

/// The last tier has nobody behind it: it is asked whatever it answered
/// before, its rejection comes back as it is, and a call does not probe or
/// bench it. (The startup probe does bench a last tier that cannot answer,
/// which only puts it on the recovery tick's list: the chain still asks it.)
#[tokio::test]
async fn a_call_never_benches_the_last_tier() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, _) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (last_tier, last) = Tier::new("gemini", Behaviour::RejectsEverything);
    let chain = chain_of(vec![head_tier, last_tier]);

    let error = chain
        .complete(&question())
        .await
        .expect_err("nothing answers");

    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(chain.is_tier_benched("anthropic"));
    assert!(!chain.is_tier_benched("gemini"));
    assert_eq!(last.calls(), 1, "the last tier was asked the question once");
}

/// A benched tier that answers the probe again goes back to work, and one
/// that does not stays benched. Nothing is asked while nothing is benched.
#[tokio::test]
async fn a_benched_tier_is_reinstated_when_it_answers_the_probe() {
    let _chain_guard = fresh_chain_guard().await;
    let (head_tier, head) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (second_tier, second) = Tier::new("copilot_sdk", Behaviour::Answers);
    let chain = chain_of(vec![head_tier, second_tier]);

    assert!(
        chain.reprobe_benched_tiers().await.is_empty(),
        "a chain with nothing benched costs no probe"
    );
    assert_eq!(head.calls(), 0);

    chain.complete(&question()).await.expect("served");
    let still_dead = chain.reprobe_benched_tiers().await;
    assert_eq!(still_dead.len(), 1, "only the benched tier is asked");
    assert_eq!(still_dead[0].provider, "anthropic");
    assert_eq!(still_dead[0].position, 0);
    assert!(still_dead[0].outcome.is_err());
    assert_eq!(
        second.calls(),
        1,
        "a tier that is not benched is not probed"
    );

    head.dead.store(false, Ordering::SeqCst);
    let recovered = chain.reprobe_benched_tiers().await;
    assert_eq!(recovered[0].outcome, Ok("anthropic-model".to_owned()));
    assert!(!chain.is_tier_benched("anthropic"));

    let served = chain.complete(&question()).await.expect("served");
    assert_eq!(
        served.model, "anthropic-model",
        "a reinstated head serves again at once, without waiting out its window"
    );
}

/// A provider that is not a chain has nothing to move to: its rejection comes
/// back unchanged and costs no probe.
#[tokio::test]
async fn a_solo_provider_gets_no_second_opinion() {
    let (solo_tier, solo) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let provider = ChatProvider::Embacle(solo_tier);

    let error = provider.complete(&question()).await.expect_err("rejected");

    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(solo.calls(), 1);
}

/// What a tool loop asks after its head failed a turn: the head alone is
/// probed, a dead one is benched, and a head already passed over is not
/// probed a second time.
#[tokio::test]
async fn the_second_opinion_on_the_head_probes_the_head_alone() {
    let _chain_guard = fresh_chain_guard().await;
    let (dead_tier, dead_head) = Tier::new("anthropic", Behaviour::RejectsEverything);
    let (behind_dead, tail) = Tier::new("copilot_sdk", Behaviour::Answers);
    let dead_led = chain_of(vec![dead_tier, behind_dead]);

    assert!(
        dead_led.bench_head_if_dead().await,
        "a head that cannot answer the probe is dead"
    );
    assert!(dead_led.is_tier_benched("anthropic"));
    assert!(
        dead_led.bench_head_if_dead().await,
        "a head already passed over sends the turn to the tail"
    );
    assert_eq!(dead_head.calls(), 1, "and is not probed again");
    assert_eq!(tail.calls(), 0, "the tier behind the head is never probed");

    let (live_tier, live_head) = Tier::new("anthropic", Behaviour::RejectsAllButThePing);
    let (behind_live, _) = Tier::new("copilot_sdk", Behaviour::Answers);
    let live_led = chain_of(vec![live_tier, behind_live]);

    assert!(
        !live_led.bench_head_if_dead().await,
        "a head that answers the probe rejected the turn itself"
    );
    assert_eq!(live_head.calls(), 1, "one probe");
    assert!(!live_led.is_tier_benched("anthropic"));

    let (solo_tier, solo) = Tier::new("anthropic", Behaviour::RejectsEverything);
    assert!(
        !ChatProvider::Embacle(solo_tier).bench_head_if_dead().await,
        "a provider with no tail has nowhere to send the turn"
    );
    assert_eq!(solo.calls(), 0, "and is not probed");
}
