// ABOUTME: Outcome evaluator — the reinforcement loop that labels due advice from real data and updates playbooks
// ABOUTME: Hybrid labeler (deterministic data heuristics + LLM judge for ambiguous cases); a background scheduler
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Outcome evaluator (P4 of coaching playbook memory)
//!
//! A background scheduler scans pending advice whose observation window has
//! closed, reads the athlete's own activity/health data over that window, and
//! labels whether the recommendation worked — then folds the label into the
//! matching playbook's reinforcement counters. Labeling is **hybrid**:
//! deterministic data rules decide the clear cases for free, and an LLM judge
//! resolves only the ambiguous ones (a small near-threshold delta).
//!
//! Three outcomes per advice: `Labeled` (recorded + advice marked labeled),
//! `Expire` (window closed with no usable data — never reinforces), and `Retry`
//! (a transient read error — left pending for the next tick).

use std::env;
use std::sync::Arc;
use std::time::Duration;

use crate::periodic::spawn_periodic;
use chrono::Utc;
use pierre_core::models::{SportType, TenantId};
use pierre_database::repositories::RecordedOutcome;
use pierre_database::RepositoryRegistry;
use pierre_llm::{judge, ChatProvider, LlmProvider};
use pierre_memory::playbooks::{LabelSource, OutcomeLabel, OutcomeMetric, PendingAdvice};
use serde::Deserialize;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Env var controlling how often the evaluator scans for due advice.
pub const OUTCOME_EVAL_INTERVAL_ENV_VAR: &str = "PIERRE_OUTCOME_EVAL_INTERVAL_SECS";
/// Default scan cadence — hourly. Outcomes resolve on a day scale, so an hourly
/// sweep is timely without being chatty.
const DEFAULT_OUTCOME_EVAL_INTERVAL_SECS: u64 = 3600;
/// How many due advice rows to process per tick.
const EVAL_BATCH_SIZE: i64 = 50;
/// Grace horizon past `due_by` after which advice that keeps failing to read its
/// window data is expired rather than retried forever. Without this backstop a
/// permanently-failing row would re-`Retry` every tick and, because the scan is
/// oldest-first and capped at `EVAL_BATCH_SIZE`, head-of-line block newer advice.
const RETRY_STALENESS_SECS: i64 = 7 * 24 * 3600;
/// Upper bound on activities pulled for one window — a window is days, so this
/// never truncates a real working set.
const ACTIVITY_SCAN_LIMIT: i64 = 200;

/// HRV dead-band in milliseconds: a smaller change is "no clear movement" and is
/// escalated to the LLM judge rather than called a success or failure.
const HRV_DEAD_BAND_MS: f64 = 3.0;
/// TSB (form) dead-band in TSS points.
const TSB_DEAD_BAND: f64 = 3.0;

/// What to do with one piece of due advice after evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdviceResolution {
    /// Labeled — record the outcome and mark the advice labeled.
    Labeled(OutcomeLabel, LabelSource),
    /// Window closed with no usable data — mark expired (never reinforces).
    Expire,
    /// Transient read failure — leave pending for the next tick.
    Retry,
}

/// Verdict shape returned by the LLM judge.
#[derive(Debug, Deserialize)]
struct JudgeVerdict {
    verdict: String,
}

/// The LLM-judge system prompt — invoked only for ambiguous (near-threshold)
/// cases, with a short data summary as the user message.
const JUDGE_PROMPT: &str = "You judge whether a fitness coaching recommendation worked, given the athlete's observed data over the window. Return ONLY JSON: {\"verdict\":\"success\"|\"failure\"|\"neutral\"}. Use \"neutral\" only when the data is genuinely inconclusive. No prose.";

// ---- Pure labeling helpers (unit-tested without a DB or LLM) ----

/// Adherence verdict from whether a matching activity landed in the window.
/// `None` => no activity data at all, so we cannot tell (the caller expires it).
#[must_use]
pub fn activity_completed_label(matched: bool, any_activity: bool) -> Option<OutcomeLabel> {
    if matched {
        Some(OutcomeLabel::Success)
    } else if any_activity {
        Some(OutcomeLabel::Failure)
    } else {
        None
    }
}

/// Outcome of a delta-metric data heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaVerdict {
    /// `None` => not enough data to compute a delta (expire).
    pub label: Option<OutcomeLabel>,
    /// `true` => within the dead-band; escalate to the LLM judge.
    pub ambiguous: bool,
}

/// Label a "did the metric move favorably" delta.
///
/// A move smaller than `dead_band` is ambiguous (Neutral + escalate); a clear
/// favorable/unfavorable move is Success/Failure. Missing endpoints yield `None`
/// (expire).
#[must_use]
pub fn delta_label(
    before: Option<f64>,
    after: Option<f64>,
    higher_is_better: bool,
    dead_band: f64,
) -> DeltaVerdict {
    let (Some(b), Some(a)) = (before, after) else {
        return DeltaVerdict {
            label: None,
            ambiguous: false,
        };
    };
    let oriented = if higher_is_better { a - b } else { b - a };
    if oriented.abs() < dead_band {
        DeltaVerdict {
            label: Some(OutcomeLabel::Neutral),
            ambiguous: true,
        }
    } else if oriented > 0.0 {
        DeltaVerdict {
            label: Some(OutcomeLabel::Success),
            ambiguous: false,
        }
    } else {
        DeltaVerdict {
            label: Some(OutcomeLabel::Failure),
            ambiguous: false,
        }
    }
}

/// Did a ramp rate stay within the safe ceiling? Inclusive at the ceiling.
///
/// The ceiling arrives widened from the metric's `f32` (lossless), so the
/// comparison is exact. The caller maps over the `Option<f64>` max ramp.
#[must_use]
pub fn ramp_within_label(max_ramp: f64, ceiling: f64) -> OutcomeLabel {
    if max_ramp <= ceiling {
        OutcomeLabel::Success
    } else {
        OutcomeLabel::Failure
    }
}

/// Label a session-count consistency window: enough sessions => Success, none =>
/// Failure, in-between => ambiguous (escalate). `expected` is the rough session
/// target for the window.
#[must_use]
pub fn consistency_label(session_count: u32, expected: u32) -> DeltaVerdict {
    if session_count >= expected.max(1) {
        DeltaVerdict {
            label: Some(OutcomeLabel::Success),
            ambiguous: false,
        }
    } else if session_count == 0 {
        DeltaVerdict {
            label: Some(OutcomeLabel::Failure),
            ambiguous: false,
        }
    } else {
        DeltaVerdict {
            label: Some(OutcomeLabel::Neutral),
            ambiguous: true,
        }
    }
}

// ---- Async evaluation (reads the data layer; may call the LLM judge) ----

/// Everything one advice evaluation needs, bundled so each per-metric helper
/// takes a single argument (and keeps argument counts in check). `tenant_id` is
/// `Copy`, so the training read can take it by value.
struct EvalCtx<'a> {
    repos: &'a RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    advice: &'a PendingAdvice,
    chat_provider: Option<&'a ChatProvider>,
}

impl EvalCtx<'_> {
    fn start(&self) -> chrono::DateTime<Utc> {
        self.advice.created_at
    }
    fn end(&self) -> chrono::DateTime<Utc> {
        self.advice.due_by
    }
}

/// Parse the advice's stringified ids back to the `Uuid` + `TenantId` the data
/// repos require. `None` (with a warning) when malformed — the caller expires.
fn parse_advice_ids(advice: &PendingAdvice) -> Option<(Uuid, TenantId)> {
    if let (Ok(uid), Ok(tid)) = (
        Uuid::parse_str(&advice.user_id),
        TenantId::parse_str(&advice.tenant_id),
    ) {
        Some((uid, tid))
    } else {
        warn!(advice_id = %advice.id, "advice has non-UUID tenant/user; expiring");
        None
    }
}

/// Evaluate one piece of due advice against the athlete's data, dispatching on
/// the outcome metric.
async fn evaluate_advice(
    advice: &PendingAdvice,
    repos: &RepositoryRegistry,
    chat_provider: Option<&ChatProvider>,
) -> AdviceResolution {
    let Some((user_id, tenant_id)) = parse_advice_ids(advice) else {
        return AdviceResolution::Expire;
    };
    let ctx = EvalCtx {
        repos,
        user_id,
        tenant_id,
        advice,
        chat_provider,
    };
    match &advice.outcome_metric {
        OutcomeMetric::ActivityCompleted { sport, .. } => {
            eval_activity_completed(&ctx, sport.as_deref()).await
        }
        OutcomeMetric::HrvDelta { .. } => eval_hrv(&ctx).await,
        OutcomeMetric::TsbDelta { .. } | OutcomeMetric::RampRateWithin { .. } => {
            eval_training(&ctx).await
        }
        OutcomeMetric::Consistency { window_days } => eval_consistency(&ctx, *window_days).await,
    }
}

/// Adherence: did a matching activity land in the window?
async fn eval_activity_completed(ctx: &EvalCtx<'_>, sport: Option<&str>) -> AdviceResolution {
    let acts = match ctx
        .repos
        .activity_cache
        .get_cached_activities(
            ctx.user_id,
            &ctx.tenant_id,
            None,
            ctx.start(),
            ctx.end(),
            ACTIVITY_SCAN_LIMIT,
        )
        .await
    {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "activity read failed; will retry");
            return AdviceResolution::Retry;
        }
    };
    let any = !acts.is_empty();
    let matched = sport.map_or(any, |slug| {
        let want = SportType::from_internal_string(slug);
        acts.iter().any(|a| a.sport_type() == &want)
    });
    resolve_data(activity_completed_label(matched, any))
}

/// Recovery: did HRV improve over the window?
async fn eval_hrv(ctx: &EvalCtx<'_>) -> AdviceResolution {
    let mut recs = match ctx
        .repos
        .recovery
        .get_recovery_metrics(ctx.user_id, &ctx.tenant_id, ctx.start(), ctx.end())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "recovery read failed; will retry");
            return AdviceResolution::Retry;
        }
    };
    recs.sort_by_key(|m| m.date);
    // HRV is captured sporadically, so the earliest/latest record in the window
    // may carry no HRV. Take the first and last records that actually have a
    // value so a boundary `None` cannot discard a computable mid-window delta.
    let before = recs.iter().find_map(|m| m.hrv_ms);
    let after = recs.iter().rev().find_map(|m| m.hrv_ms);
    let verdict = delta_label(before, after, true, HRV_DEAD_BAND_MS);
    self_or_judge(ctx, verdict, "HRV (ms)", before, after).await
}

/// Consistency: did the athlete hold a reasonable session cadence?
async fn eval_consistency(ctx: &EvalCtx<'_>, window_days: u8) -> AdviceResolution {
    let acts = match ctx
        .repos
        .activity_cache
        .get_cached_activities(
            ctx.user_id,
            &ctx.tenant_id,
            None,
            ctx.start(),
            ctx.end(),
            ACTIVITY_SCAN_LIMIT,
        )
        .await
    {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "activity read failed; will retry");
            return AdviceResolution::Retry;
        }
    };
    // ~2 sessions per week target over the window.
    let expected = ((f64::from(window_days) / 7.0) * 2.0).round();
    let count = u32::try_from(acts.len()).unwrap_or(u32::MAX);
    let verdict = consistency_label(count, expected_as_u32(expected));
    self_or_judge(
        ctx,
        verdict,
        "sessions",
        Some(f64::from(count)),
        Some(expected),
    )
    .await
}

/// TSB / ramp-rate: read training-history endpoints over the window.
async fn eval_training(ctx: &EvalCtx<'_>) -> AdviceResolution {
    let from = ctx.start().date_naive();
    let to = ctx.end().date_naive();
    let hist = match ctx
        .repos
        .training_history
        .get_training_history(ctx.tenant_id, ctx.user_id, from, to)
        .await
    {
        Ok(h) => h,
        Err(e) => {
            warn!(error = %e, "training-history read failed; will retry");
            return AdviceResolution::Retry;
        }
    };
    match &ctx.advice.outcome_metric {
        OutcomeMetric::TsbDelta { .. } => {
            let before = hist.first().map(|d| d.tsb);
            let after = hist.last().map(|d| d.tsb);
            let verdict = delta_label(before, after, true, TSB_DEAD_BAND);
            self_or_judge(ctx, verdict, "TSB", before, after).await
        }
        OutcomeMetric::RampRateWithin { ceiling } => {
            let max_ramp = hist
                .iter()
                .filter_map(|d| d.ramp_rate)
                .fold(None, |acc: Option<f64>, r| {
                    Some(acc.map_or(r, |m| m.max(r)))
                });
            resolve_data(max_ramp.map(|r| ramp_within_label(r, f64::from(*ceiling))))
        }
        // Unreachable: this fn is only dispatched for the two training metrics.
        _ => AdviceResolution::Expire,
    }
}

/// Clamp a small non-negative `f64` session target to `u32`.
fn expected_as_u32(expected: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        expected.max(0.0) as u32
    }
}

/// Map a heuristic label (`None` => no data) to a resolution; data-sourced.
fn resolve_data(label: Option<OutcomeLabel>) -> AdviceResolution {
    label.map_or(AdviceResolution::Expire, |l| {
        AdviceResolution::Labeled(l, LabelSource::DataHeuristic)
    })
}

/// Resolve a delta verdict: a clear verdict is data-sourced; an ambiguous one is
/// escalated to the LLM judge (falling back to the heuristic when no provider).
async fn self_or_judge(
    ctx: &EvalCtx<'_>,
    verdict: DeltaVerdict,
    metric_label: &str,
    before: Option<f64>,
    after: Option<f64>,
) -> AdviceResolution {
    let Some(tentative) = verdict.label else {
        return AdviceResolution::Expire;
    };
    if !verdict.ambiguous {
        return AdviceResolution::Labeled(tentative, LabelSource::DataHeuristic);
    }
    run_judge(ctx, metric_label, before, after).await.map_or(
        AdviceResolution::Labeled(tentative, LabelSource::DataHeuristic),
        |label| AdviceResolution::Labeled(label, LabelSource::LlmJudge),
    )
}

/// Ask the LLM judge to resolve an ambiguous case. Returns `None` when there is
/// no provider or the call fails.
async fn run_judge(
    ctx: &EvalCtx<'_>,
    metric_label: &str,
    before: Option<f64>,
    after: Option<f64>,
) -> Option<OutcomeLabel> {
    let provider = ctx.chat_provider?;
    let summary = format!(
        "Recommendation: a '{}' intervention for a '{}' trigger. Observed {metric_label}: {} -> {} over the window. Did it work?",
        ctx.advice.intervention.kind.as_str(),
        ctx.advice.trigger.kind.as_str(),
        before.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.1}")),
        after.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.1}")),
    );
    match judge::ask_for_json::<JudgeVerdict>(
        provider as &dyn LlmProvider,
        JUDGE_PROMPT,
        &summary,
        0.0,
    )
    .await
    {
        Ok(v) => Some(OutcomeLabel::parse_lenient(&v.verdict)),
        Err(e) => {
            warn!(error = %e, "outcome LLM judge failed; falling back to heuristic");
            None
        }
    }
}

// ---- Scheduler ----

/// Spawn the background outcome evaluator.
///
/// Mirrors the followup scheduler: a `tokio::time::interval` loop that skips the
/// immediate first tick (so a restart doesn't slam the DB), then each tick scans
/// due advice, evaluates it, and records the outcome. Best-effort — every error
/// is logged, never propagated. Needs the shared [`ChatProvider`] singleton for
/// the LLM judge; without it, ambiguous cases fall back to the data heuristic.
pub fn spawn_outcome_evaluator(
    repos: Arc<RepositoryRegistry>,
    chat_provider: Option<Arc<ChatProvider>>,
) {
    let interval_secs = env::var(OUTCOME_EVAL_INTERVAL_ENV_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_OUTCOME_EVAL_INTERVAL_SECS);
    spawn_periodic(
        "outcome evaluator",
        Duration::from_secs(interval_secs),
        move || {
            let repos = Arc::clone(&repos);
            let chat_provider = chat_provider.clone();
            async move {
                run_one_sweep(&repos, chat_provider.as_deref()).await;
                Ok(())
            }
        },
    );
}

/// One scan-and-label sweep over the currently-due advice.
async fn run_one_sweep(repos: &RepositoryRegistry, chat_provider: Option<&ChatProvider>) {
    let now = Utc::now().timestamp();
    let due = match repos
        .playbooks
        .due_pending_advice(now, EVAL_BATCH_SIZE)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, "outcome evaluator: due-advice scan failed");
            return;
        }
    };
    let mut labeled = 0_usize;
    for advice in &due {
        if process_one_advice(advice, repos, chat_provider).await {
            labeled += 1;
        }
    }
    if labeled > 0 {
        debug!(
            labeled,
            scanned = due.len(),
            "outcome evaluator sweep complete"
        );
    }
}

/// Resolve and persist one piece of advice. Returns `true` when it was labeled.
async fn process_one_advice(
    advice: &PendingAdvice,
    repos: &RepositoryRegistry,
    chat_provider: Option<&ChatProvider>,
) -> bool {
    match evaluate_advice(advice, repos, chat_provider).await {
        AdviceResolution::Labeled(label, source) => {
            record_and_mark(repos, advice, label, source).await
        }
        AdviceResolution::Expire => {
            expire_advice(repos, advice).await;
            false
        }
        AdviceResolution::Retry => {
            // Expire a row that has kept failing well past its window so a
            // permanent read failure cannot head-of-line block the oldest-first
            // scan forever; a normal transient failure just waits for next tick.
            if Utc::now().timestamp() - advice.due_by.timestamp() > RETRY_STALENESS_SECS {
                expire_advice(repos, advice).await;
            }
            false
        }
    }
}

/// Mark one piece of advice expired (tenant-scoped); log-and-swallow on error.
async fn expire_advice(repos: &RepositoryRegistry, advice: &PendingAdvice) {
    if let Err(e) = repos
        .playbooks
        .mark_advice_expired(&advice.tenant_id, &advice.id)
        .await
    {
        error!(error = %e, "failed to mark advice expired");
    }
}

/// Atomically record an outcome into its playbook and mark the advice labeled.
///
/// Uses the one-transaction repository path
/// ([`pierre_database::repositories::PlaybookRepository::record_outcome_and_label`]),
/// then emits the `playbook.outcome_labeled` notify event on success. Returns
/// `true` only when the outcome was durably recorded.
async fn record_and_mark(
    repos: &RepositoryRegistry,
    advice: &PendingAdvice,
    label: OutcomeLabel,
    source: LabelSource,
) -> bool {
    let outcome = RecordedOutcome {
        tenant_id: &advice.tenant_id,
        user_id: &advice.user_id,
        coach_slug: advice.coach_slug.as_deref(),
        trigger: &advice.trigger,
        intervention: &advice.intervention,
        outcome_metric: &advice.outcome_metric,
        label,
        at: Utc::now(),
    };
    match repos
        .playbooks
        .record_outcome_and_label(&outcome, &advice.id, source)
        .await
    {
        Ok(_playbook_id) => {
            info!(
                target: "notify",
                event = "playbook.outcome_labeled",
                tenant_id = %advice.tenant_id,
                user_id = %advice.user_id,
                label = label.as_str(),
                source = source.as_str(),
                "labeled coaching advice outcome"
            );
            true
        }
        Err(e) => {
            error!(error = %e, "failed to record and label playbook outcome");
            false
        }
    }
}
