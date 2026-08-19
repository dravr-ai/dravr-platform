// ABOUTME: Advice capture — turns a coach's concrete recommendation into a PendingAdvice awaiting its outcome
// ABOUTME: Strategy pattern (v1 = heuristic-gated LLM extraction); runs as a background task after a turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Advice capture (P3 of coaching playbook memory)
//!
//! When the coach makes a concrete, checkable recommendation, we record a
//! [`PendingAdvice`] so the outcome evaluator can later label whether it worked
//! and reinforce the matching playbook. Capture is a swappable
//! [`AdviceCaptureStrategy`]; v1 ships [`HeuristicGatedLlmExtraction`] — a cheap
//! keyword/length gate followed by a bounded LLM extraction. The two alternative
//! strategies (an explicit `record_recommendation` tool, and every-turn
//! extraction) are documented in `DRAVR-BACKLOG.md` and implement the same trait.
//!
//! Like memory extraction, capture runs **after** the reply has been flushed to
//! the user and logs-and-swallows every error — it never blocks or fails a turn.
//!
//! The extraction prompt is compiled in here for v1. Promoting it to a
//! dravr-contremaitre hot-reloadable prompt is a tracked follow-up.

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppError;
use pierre_database::repositories::PlaybookRepository;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, LlmProvider};
use pierre_memory::playbooks::{
    sanitize_sport_slug, AdviceStatus, Band, Intervention, InterventionKind, MetricBaseline,
    OutcomeMetric, PendingAdvice, TriggerKind, TriggerPattern,
};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Cap on concurrent background advice captures — the same backpressure idiom as
/// memory extraction so a burst of turns cannot fan out unbounded LLM calls.
const MAX_CONCURRENT_CAPTURES: usize = 16;

/// Minimum reply length (bytes) before the gate even considers running the
/// extractor — greetings and one-liners are never recommendations.
const MIN_REPLY_LEN: usize = 40;

/// Clamp range for the model-provided observation window, in days.
const MIN_WINDOW_DAYS: u8 = 1;
const MAX_WINDOW_DAYS: u8 = 30;

/// Default acute:chronic ceiling for a `ramp_rate_within` outcome metric when
/// the model does not (and cannot) specify one.
///
/// 1.3 is not borrowed from the retired injury-prediction literature: because
/// `tsb == ctl - atl`, an acute:chronic ratio of 1.3 is exactly where form
/// crosses -30% of CTL — the edge of `FormBand::DeepFatigue`. The ceiling names
/// the same band boundary every other surface bands on, in ratio form.
const DEFAULT_RAMP_CEILING: f32 = 1.3;

/// Global semaphore bounding concurrent captures.
static CAPTURE_PERMITS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CAPTURES)));

/// English + French cues the recommendation gate scans for. The platform is
/// French-first, so both are covered; the list is deliberately broad — the LLM
/// extraction is the real filter, this only screens out obvious non-advice.
const RECOMMENDATION_CUES: &[&str] = &[
    // English
    "recommend",
    "suggest",
    "try ",
    "let's",
    "should",
    "add ",
    "aim for",
    "focus on",
    "i'd ",
    "go for",
    "this week",
    "tomorrow",
    "next week",
    "keep ",
    "start ",
    // French
    "recommande",
    "suggère",
    "essaie",
    "essaye",
    "devrais",
    "ajoute",
    "vise ",
    "concentre",
    "demain",
    "cette semaine",
    "semaine prochaine",
    "commence",
    "garde",
];

/// The compiled-in advice-extraction prompt. Asks the model to return a JSON
/// array of structured, checkable recommendations (or `[]`).
const ADVICE_EXTRACTION_PROMPT: &str = r#"You analyze a fitness coaching exchange and extract any CONCRETE, actionable recommendation the coach made that can later be checked against the athlete's own activity/health data.

Return ONLY a JSON array (possibly empty). Each element:
{
  "trigger_kind": one of ["motivation_dip","hrv_drop","load_ramp","plateau","travel","pre_planned","other"],
  "trigger_sport": optional sport slug like "run","ride","swim", or null,
  "trigger_magnitude": one of ["low","moderate","high"],
  "intervention_kind": one of ["easy_block","add_tempo","add_threshold","minimum_viable","reduce_volume","rest_day","comm_style_terse","comm_style_analytical","other"],
  "intervention_magnitude": optional integer (days, or sessions/week), or null,
  "outcome_metric": one of ["activity_completed","hrv_delta","tsb_delta","ramp_rate_within","consistency"],
  "outcome_sport": optional sport slug for activity_completed, or null,
  "window_days": integer 1-30 — how long until we can judge whether it worked
}

Rules:
- Emit an element ONLY when the coach gave a SPECIFIC recommendation (do X), not general chat, questions, or data summaries.
- outcome_metric = the most direct measurable signal of success: a suggested workout -> activity_completed; recovery advice -> hrv_delta; load/injury caution -> ramp_rate_within or tsb_delta; adherence/habit -> consistency.
- Return [] when there is no concrete, checkable recommendation.
- Output ONLY the JSON array, no prose, no code fence."#;

/// Owned, `Clone` snapshot of a finished turn for the background capture task.
///
/// `coach_slug`/`tenant_id`/`user_id` scope the resulting playbook; the two
/// message texts feed the extractor.
#[derive(Debug, Clone)]
pub struct CapturedTurn {
    /// Tenant that owns the conversation (and will own the playbook).
    pub tenant_id: String,
    /// User the advice was given to.
    pub user_id: String,
    /// Coach persona slug, or `None` for a coach-agnostic playbook.
    pub coach_slug: Option<String>,
    /// The user message that started the turn.
    pub user_message: String,
    /// The assistant reply that completed the turn.
    pub assistant_reply: String,
    /// The assistant message id, for advice provenance.
    pub source_msg_id: Option<String>,
}

/// Pluggable mechanism for turning a finished turn into zero or more
/// [`PendingAdvice`] records. Implementations must be cheap to call on every
/// turn (gate first, spend LLM tokens only when warranted).
#[async_trait]
pub trait AdviceCaptureStrategy: Send + Sync {
    /// Extract any checkable recommendations from `turn`. Best-effort: returns
    /// an empty vec (never errors) when there is nothing to capture or the
    /// extraction fails.
    async fn capture(&self, turn: &CapturedTurn, provider: &ChatProvider) -> Vec<PendingAdvice>;
}

/// v1 strategy: a cheap heuristic gate, then a bounded LLM extraction.
///
/// Catches advice the model did not self-tag while spending tokens only on
/// turns that look like recommendations.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicGatedLlmExtraction;

/// Raw advice shape returned by the extraction LLM (all stringly-typed; mapped
/// to the domain enums via `parse_lenient`).
#[derive(Debug, Deserialize)]
struct RawAdvice {
    trigger_kind: String,
    trigger_sport: Option<String>,
    trigger_magnitude: String,
    intervention_kind: String,
    intervention_magnitude: Option<i32>,
    outcome_metric: String,
    outcome_sport: Option<String>,
    window_days: u8,
}

/// Cheap, imperfect gate: is this reply worth an extraction call?
///
/// The LLM is the real filter (it returns `[]` when there is no actionable
/// advice), so this only screens out the obvious non-recommendations (greetings,
/// data dumps, questions). Covers both English and French cues since the
/// platform is French-first.
#[must_use]
pub fn looks_like_recommendation(reply: &str) -> bool {
    if reply.trim().len() < MIN_REPLY_LEN {
        return false;
    }
    let lower = reply.to_lowercase();
    RECOMMENDATION_CUES.iter().any(|cue| lower.contains(cue))
}

/// Map a raw extracted advice into a typed [`PendingAdvice`].
///
/// Returns `None` when it is too malformed to act on. Pure so it can be
/// unit-tested without an LLM.
#[must_use]
pub fn raw_to_pending(
    raw: &RawAdvicePublic,
    turn: &CapturedTurn,
    now: DateTime<Utc>,
) -> Option<PendingAdvice> {
    let window_days = raw.window_days.clamp(MIN_WINDOW_DAYS, MAX_WINDOW_DAYS);
    let trigger = TriggerPattern {
        kind: TriggerKind::parse_lenient(&raw.trigger_kind),
        sport: sanitize_sport_slug(raw.trigger_sport.as_deref()),
        magnitude: Band::parse_lenient(&raw.trigger_magnitude),
    };
    let intervention = Intervention {
        kind: InterventionKind::parse_lenient(&raw.intervention_kind),
        magnitude: raw.intervention_magnitude,
    };
    let outcome_metric = metric_from_raw(
        &raw.outcome_metric,
        window_days,
        sanitize_sport_slug(raw.outcome_sport.as_deref()),
    );
    let due_by = now + chrono::Duration::days(i64::from(window_days));
    Some(PendingAdvice {
        id: Uuid::new_v4().to_string(),
        tenant_id: turn.tenant_id.clone(),
        user_id: turn.user_id.clone(),
        coach_slug: turn.coach_slug.clone(),
        playbook_id: None,
        trigger,
        intervention,
        outcome_metric,
        baseline: MetricBaseline { captured_at: now },
        due_by,
        status: AdviceStatus::Pending,
        label: None,
        label_source: None,
        source_msg_id: turn.source_msg_id.clone(),
        created_at: now,
    })
}

/// Public mirror of the LLM's raw advice shape so the pure mapping
/// ([`raw_to_pending`]) is unit-testable from an external test crate.
#[derive(Debug, Clone)]
pub struct RawAdvicePublic {
    /// Trigger-kind slug.
    pub trigger_kind: String,
    /// Optional trigger sport slug.
    pub trigger_sport: Option<String>,
    /// Trigger magnitude slug.
    pub trigger_magnitude: String,
    /// Intervention-kind slug.
    pub intervention_kind: String,
    /// Optional intervention magnitude.
    pub intervention_magnitude: Option<i32>,
    /// Outcome-metric slug.
    pub outcome_metric: String,
    /// Optional outcome sport slug (for `activity_completed`).
    pub outcome_sport: Option<String>,
    /// Observation window in days (clamped on use).
    pub window_days: u8,
}

impl From<RawAdvice> for RawAdvicePublic {
    fn from(r: RawAdvice) -> Self {
        Self {
            trigger_kind: r.trigger_kind,
            trigger_sport: r.trigger_sport,
            trigger_magnitude: r.trigger_magnitude,
            intervention_kind: r.intervention_kind,
            intervention_magnitude: r.intervention_magnitude,
            outcome_metric: r.outcome_metric,
            outcome_sport: r.outcome_sport,
            window_days: r.window_days,
        }
    }
}

/// Map an outcome-metric slug + window + sport to a typed [`OutcomeMetric`].
/// Unknown slugs fall back to `ActivityCompleted` — the simplest, always-valid
/// adherence signal.
fn metric_from_raw(kind: &str, window_days: u8, sport: Option<String>) -> OutcomeMetric {
    match kind {
        "hrv_delta" => OutcomeMetric::HrvDelta { window_days },
        "tsb_delta" => OutcomeMetric::TsbDelta { window_days },
        "consistency" => OutcomeMetric::Consistency { window_days },
        "ramp_rate_within" => OutcomeMetric::RampRateWithin {
            ceiling: DEFAULT_RAMP_CEILING,
        },
        _ => OutcomeMetric::ActivityCompleted { window_days, sport },
    }
}

#[async_trait]
impl AdviceCaptureStrategy for HeuristicGatedLlmExtraction {
    async fn capture(&self, turn: &CapturedTurn, provider: &ChatProvider) -> Vec<PendingAdvice> {
        if !looks_like_recommendation(&turn.assistant_reply) {
            debug!("advice capture: reply not recommendation-like; gate skipped LLM");
            return Vec::new();
        }
        let raw = match run_advice_extraction(provider, turn).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "advice extraction LLM call failed; capturing nothing");
                return Vec::new();
            }
        };
        let now = Utc::now();
        raw.into_iter()
            .filter_map(|r| raw_to_pending(&RawAdvicePublic::from(r), turn, now))
            .collect()
    }
}

/// Call the extraction LLM and parse its response into raw advice records.
async fn run_advice_extraction(
    provider: &ChatProvider,
    turn: &CapturedTurn,
) -> Result<Vec<RawAdvice>, AppError> {
    let user_payload = format!(
        "User turn:\n{}\n\nCoach reply:\n{}\n\nReturn the JSON array only.",
        turn.user_message, turn.assistant_reply
    );
    let request = ChatRequest::new(vec![
        ChatMessage::system(ADVICE_EXTRACTION_PROMPT),
        ChatMessage::user(&user_payload),
    ])
    .with_temperature(0.1);
    let response = LlmProvider::complete(provider, &request)
        .await
        .map_err(|e| AppError::external_service("advice-extractor", format!("LLM: {e}")))?;
    Ok(parse_raw_advice(&response.content))
}

/// Lenient JSON-array parse: raw array, then a fenced block, then the first
/// `[`..last `]` span. Unparseable input yields an empty vec, never an error —
/// the same forgiving contract memory extraction uses.
fn parse_raw_advice(response: &str) -> Vec<RawAdvice> {
    if let Ok(parsed) = serde_json::from_str::<Vec<RawAdvice>>(response) {
        return parsed;
    }
    if let Some(start) = response.find("```json") {
        let after = &response[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            if let Ok(parsed) = serde_json::from_str::<Vec<RawAdvice>>(after[..end].trim()) {
                return parsed;
            }
        }
    }
    if let (Some(s), Some(e)) = (response.find('['), response.rfind(']')) {
        if s <= e {
            if let Ok(parsed) = serde_json::from_str::<Vec<RawAdvice>>(&response[s..=e]) {
                return parsed;
            }
        }
    }
    warn!(
        length = response.len(),
        "advice extractor returned non-JSON; ignoring"
    );
    Vec::new()
}

/// Fire-and-forget advice capture.
///
/// Gates + extracts on a bounded background task, then persists each
/// [`PendingAdvice`]. Mirrors `spawn_extract_for_turn` — needs the shared
/// [`ChatProvider`] singleton (skips cleanly when absent), and logs and
/// swallows every error.
pub fn spawn_capture_advice(
    playbook_repo: Arc<dyn PlaybookRepository>,
    chat_provider: Option<Arc<ChatProvider>>,
    strategy: Arc<dyn AdviceCaptureStrategy>,
    turn: CapturedTurn,
) {
    let permits = Arc::clone(&CAPTURE_PERMITS);
    tokio::spawn(async move {
        let Ok(permit) = permits.acquire_owned().await else {
            debug!("advice capture skipped: semaphore closed");
            return;
        };
        let Some(provider) = &chat_provider else {
            debug!("advice capture skipped: no chat_provider singleton wired");
            return;
        };
        let advice = strategy.capture(&turn, provider.as_ref()).await;
        let mut persisted = 0_usize;
        for a in &advice {
            match playbook_repo.insert_pending_advice(a).await {
                Ok(()) => persisted += 1,
                Err(e) => error!(error = %e, "failed to persist pending advice"),
            }
        }
        if persisted > 0 {
            info!(
                target: "notify",
                event = "playbook.advice_captured",
                tenant_id = %turn.tenant_id,
                user_id = %turn.user_id,
                count = persisted,
                "captured coaching advice as pending"
            );
        }
        drop(permit);
    });
}
