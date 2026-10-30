// ABOUTME: Post-turn usage recording — the write side of the same counters the pre-turn check reads
// ABOUTME: Recorded under the athlete's own tenant so messaging usage depletes the budget web enforces

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage recording for a completed turn.
//!
//! The write half of [`crate::quota_policy`]. Both halves take the athlete's
//! own tenant and both are driven from [`crate::turn_service::execute`], which
//! is what keeps the counter a cap is measured against and the counter a turn
//! increments from drifting apart. Recording under the channel-owner tenant is
//! precisely how messaging usage became invisible to every quota read
//! (registre#9), so the tenant is a parameter of the turn, not of the surface.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::tokens::estimate_chat_tokens;
use pierre_llm::TokenUsage;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use pierre_services::usage_counter::UsageCounterService;
use tracing::{info, warn};
use uuid::Uuid;

use crate::envelope::TurnEnvelope;
use crate::ChatPipelineContext;

/// Per-turn dimensions that drive the scoped counter increments.
///
/// Mirrors [`crate::quota_policy::PreChatScope`] on the read side so
/// pre-check and post-increment stay in lockstep.
#[derive(Debug, Default, Clone)]
pub struct UsageIncrementScope<'a> {
    /// `chat_conversations.id` — drives the daily per-conversation message
    /// counter the pre-turn check enforces against
    /// `max_messages_per_conversation`.
    pub conversation_id: Option<&'a str>,
    /// `agents.id` — drives the daily per-agent message counter the pre-turn
    /// check enforces against `max_messages_per_agent_per_day`.
    pub agent_id: Option<&'a str>,
}

/// Weight, in percent of a fresh token, that a prompt-cache read carries
/// against the token quotas (`daily_tokens`, `weekly_tokens`).
///
/// Zero by product decision (carnet#691): a cache read is context the provider
/// already holds and bills at a steep discount, and on the Copilot ACP path it
/// is mostly the vendor's own preamble re-served on every tool-loop iteration.
/// One five-tool activity question reported 455,493 prompt tokens of which
/// 378,608 were cache reads; charged at full weight that single turn spent 91%
/// of the Starter plan's daily budget. Fresh input, cache writes and output are
/// charged in full. The `llm_usage` cost rows are unaffected — they record the
/// read split as reported and price it with the model's own cache multiplier.
pub(crate) const CACHE_READ_QUOTA_WEIGHT_PERCENT: i64 = 0;

/// Tokens one provider-reported usage charges against the token quotas.
///
/// `prompt_tokens` is gross — every embacle provider folds cache reads and
/// writes into it — so the cache-read share is carved back out and re-added at
/// [`CACHE_READ_QUOTA_WEIGHT_PERCENT`]. The read is clamped to the prompt so a
/// provider over-reporting it can never drive the charge below the output.
/// A provider that reports no cache split charges the gross prompt, as it
/// always has.
#[must_use]
pub(crate) fn quota_tokens_for_usage(usage: &TokenUsage) -> i64 {
    let prompt = i64::from(usage.prompt_tokens);
    let cached_read = usage
        .cached_read_tokens
        .map_or(0, i64::from)
        .clamp(0, prompt);
    let charged_prompt = prompt - cached_read + cached_read * CACHE_READ_QUOTA_WEIGHT_PERCENT / 100;
    charged_prompt + i64::from(usage.completion_tokens)
}

/// Tokens a completed turn charges against the token quotas.
///
/// Prefers real provider-reported counts, weighed by
/// [`quota_tokens_for_usage`]. When the provider does not report usage
/// (CLI-based providers such as Copilot headless), falls back to
/// character-based estimation on the athlete's input for the prompt side and
/// on the persisted assistant row — the same bytes the athlete was sent — for
/// the completion side.
#[must_use]
pub(crate) fn quota_tokens_from_envelope(envelope: &TurnEnvelope, user_content: &str) -> i64 {
    envelope.telemetry.usage.as_ref().map_or_else(
        || {
            let (prompt, completion) =
                estimate_chat_tokens(user_content, &envelope.assistant.message.content);
            i64::from(prompt) + i64::from(completion)
        },
        quota_tokens_for_usage,
    )
}

/// Increment the daily/weekly message and token counters for one served turn,
/// plus the per-conversation and per-agent counters when their ids are present
/// in [`UsageIncrementScope`].
///
/// The same dimension keys the pre-turn check reads
/// (`conversation_messages:<conv>`, `daily_coach_messages:<agent>`) are
/// written here. Failures are logged rather than propagated: the athlete
/// already has their reply, and losing a counter must not turn a delivered
/// turn into an error.
pub async fn increment_usage_counters_scoped(
    ctx: &ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
    total_tokens: i64,
    scope: &UsageIncrementScope<'_>,
) {
    // Record against tier defaults even when admin config is absent, so the
    // counters the always-on enforcement path reads keep accumulating.
    let compiled_defaults: &dyn AdminConfigLookup = default_admin_config();
    let admin_config: &dyn AdminConfigLookup =
        ctx.admin_config.as_deref().unwrap_or(compiled_defaults);

    let usage_svc = UsageCounterService::new(ctx.repos.usage_counters.as_ref(), admin_config);
    let tenant_str = tenant_id.to_string();
    let user_str = user_id.to_string();

    increment_base_counters(&usage_svc, &tenant_str, &user_str, total_tokens).await;
    increment_scoped_counters(&usage_svc, &tenant_str, &user_str, scope).await;
}

/// Bump the global daily/weekly message and token counters.
async fn increment_base_counters(
    usage_svc: &UsageCounterService<'_>,
    tenant_id: &str,
    user_id: &str,
    total_tokens: i64,
) {
    let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
    if total_tokens > 0 {
        counters.push(("daily_tokens", total_tokens));
        counters.push(("weekly_tokens", total_tokens));
    }

    for (counter_type, amount) in counters {
        let outcome = usage_svc
            .increment(tenant_id, user_id, counter_type, amount)
            .await;
        note_counter_charge(counter_type, None, amount, &outcome);
    }
}

/// Log what a turn charged against one counter and where the counter stands.
///
/// Without it the 80% and burst notices a turn triggers can only be inferred,
/// since nothing else says what the turn charged against `daily_tokens` or
/// what the counter read afterwards (carnet#1194). `current` is the value the
/// increment returned, so the line and the counter the next pre-turn check
/// reads agree. The caps are not repeated here: the pre-turn ladder resolved
/// them for this turn already and refused it if a hard one was breached.
fn note_counter_charge(
    counter_type: &str,
    dimension: Option<&str>,
    charged: i64,
    outcome: &AppResult<i64>,
) {
    match outcome {
        Ok(current) => info!(
            counter_type,
            dimension, charged, current, "quota counter charged for the turn"
        ),
        Err(e) => warn!("Failed to increment {counter_type} counter: {e}"),
    }
}

/// Bump the per-conversation and per-agent dimensioned counters.
async fn increment_scoped_counters(
    usage_svc: &UsageCounterService<'_>,
    tenant_id: &str,
    user_id: &str,
    scope: &UsageIncrementScope<'_>,
) {
    if let Some(conv_id) = scope.conversation_id {
        let outcome = usage_svc
            .increment_with_dimension(tenant_id, user_id, "conversation_messages", conv_id, 1)
            .await;
        note_counter_charge("conversation_messages", Some(conv_id), 1, &outcome);
    }

    if let Some(agent_id) = scope.agent_id {
        let outcome = usage_svc
            .increment_with_dimension(tenant_id, user_id, "daily_coach_messages", agent_id, 1)
            .await;
        note_counter_charge("daily_coach_messages", Some(agent_id), 1, &outcome);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex, PoisonError};

    use pierre_core::errors::AppError;
    use tracing::field::{Field, Visit};
    use tracing::{callsite, subscriber, Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::Registry;

    use super::*;

    /// The fields of every quota-charge line logged while installed.
    #[derive(Clone, Default)]
    struct ChargeLines(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

    struct FieldText<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for FieldText<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl<S: Subscriber> Layer<S> for ChargeLines {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldText(&mut fields));
            if fields.get("message").map(String::as_str)
                == Some("quota counter charged for the turn")
            {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(fields);
            }
        }
    }

    fn charge_lines_while(run: impl FnOnce()) -> Vec<BTreeMap<String, String>> {
        let lines = ChargeLines::default();
        subscriber::with_default(Registry::default().with(lines.clone()), || {
            callsite::rebuild_interest_cache();
            run();
        });
        // Bound first: the guard is a temporary the tail expression would
        // drop while the clone still borrows it.
        let taken = lines
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        taken
    }

    /// The line says what the turn charged and what the counter reads after,
    /// so the 80% and burst notices are explained by the log rather than
    /// inferred (carnet#1194).
    #[test]
    fn a_charged_counter_logs_the_charge_and_where_it_stands() {
        let lines =
            charge_lines_while(|| note_counter_charge("daily_tokens", None, 70_973, &Ok(146_249)));
        assert_eq!(lines.len(), 1, "one line per counter: {lines:?}");
        let line = lines.first().expect("one charge line"); // Safe: asserted above
        let field = |name: &str| line.get(name).map(String::as_str);
        assert_eq!(field("counter_type"), Some("daily_tokens"));
        assert_eq!(field("charged"), Some("70973"));
        assert_eq!(field("current"), Some("146249"));
        assert_eq!(field("dimension"), None);
    }

    /// A scoped counter keeps its type and its dimension apart, so a query on
    /// the exact `counter_type` finds it.
    #[test]
    fn a_scoped_counter_logs_its_dimension_apart_from_its_type() {
        let lines = charge_lines_while(|| {
            note_counter_charge("conversation_messages", Some("conv-1"), 1, &Ok(4));
        });
        assert_eq!(lines.len(), 1, "one line per counter: {lines:?}");
        let line = lines.first().expect("one charge line"); // Safe: asserted above
        assert_eq!(
            line.get("counter_type").map(String::as_str),
            Some("conversation_messages")
        );
        assert_eq!(line.get("dimension").map(String::as_str), Some("conv-1"));
    }

    /// A counter that could not be incremented charged nothing, so no charge
    /// line claims it did.
    #[test]
    fn a_failed_increment_logs_no_charge() {
        let lines = charge_lines_while(|| {
            note_counter_charge("daily_tokens", None, 1, &Err(AppError::internal("db down")));
        });
        assert!(
            lines.is_empty(),
            "no charge line on a failed increment: {lines:?}"
        );
    }

    /// The 2026-10-01 incident turn: five Copilot ACP tool-loop calls,
    /// split by the usage line as cachedRead 378,608 + cachedWrite 76,873
    /// + 12 fresh.
    fn incident_usage(completion: u32) -> TokenUsage {
        TokenUsage::new(455_493, completion, 455_493 + completion)
            .with_cache(Some(378_608), Some(76_873))
    }

    #[test]
    fn incident_turn_charges_fresh_plus_cache_write_plus_output() {
        assert_eq!(
            quota_tokens_for_usage(&incident_usage(1_204)),
            76_885 + 1_204
        );
    }

    #[test]
    fn provider_without_cache_split_charges_the_gross_prompt() {
        let usage = TokenUsage::new(455_493, 1_204, 456_697);
        assert_eq!(quota_tokens_for_usage(&usage), 456_697);
    }

    #[test]
    fn cache_write_only_turn_charges_in_full() {
        let usage = TokenUsage::new(62_564, 40, 62_604).with_cache(Some(0), Some(62_564));
        assert_eq!(quota_tokens_for_usage(&usage), 62_604);
    }

    #[test]
    fn over_reported_cache_read_never_charges_below_output() {
        let usage = TokenUsage::new(1_000, 25, 1_025).with_cache(Some(5_000), None);
        assert_eq!(quota_tokens_for_usage(&usage), 25);
    }
}
