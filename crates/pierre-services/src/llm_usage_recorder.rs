// ABOUTME: The one sink that persists an LLM call's usage as an llm_usage row, priced by the tenant-aware registry
// ABOUTME: Used by the tool loops and every side call — language, extraction, compaction, judge, capture, re-asks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `llm_usage` recorder.
//!
//! One row per LLM call, keyed on the turn the call belongs to and typed by
//! `call_type`, so a turn's cost is the sum of its rows and a side call is
//! counted apart from the reply it serves. Deliberately fire-and-forget: a
//! failed usage row is logged, never propagated, because losing an
//! accounting row is strictly better than failing the athlete's turn over it.
//!
//! It lives here, below the chat pipeline, because the calls it records do:
//! memory extraction, advice capture and the outcome evaluator run in this
//! crate, and the resume sweep and the evaluator build their recorders without
//! a turn in hand.
//!
//! Each write is spawned on the [`TaskTracker`] its [`LlmUsageSink`] carries —
//! in the server, the handle of its in-flight turn tracker — so the SIGTERM
//! drain awaits a row a turn's last side call wrote instead of the runtime's
//! teardown cutting it, and a write that panics is logged rather than lost
//! with its dropped handle.

use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use embacle::pricing::TokenCounts;
use futures_util::FutureExt;
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::ConversationTurnId;
use pierre_database::repositories::LlmUsageRepository;
use pierre_llm::call_record::{LlmCallRecord, LlmCallRecorder};
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

use crate::pricing::GLOBAL_PRICING_REGISTRY;

/// Where `llm_usage` rows go: the repository they are written to, and the
/// tracker each write is spawned on so the shutdown drain awaits it.
#[derive(Clone)]
pub struct LlmUsageSink {
    /// The `llm_usage` table.
    repo: Arc<dyn LlmUsageRepository>,
    /// The drain-tracked spawner; clones share one count.
    writes: TaskTracker,
}

impl fmt::Debug for LlmUsageSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmUsageSink")
            .field("writes", &self.writes)
            .finish_non_exhaustive()
    }
}

impl LlmUsageSink {
    /// Rows written to `repo`, each write spawned on `writes`.
    #[must_use]
    pub const fn new(repo: Arc<dyn LlmUsageRepository>, writes: TaskTracker) -> Self {
        Self { repo, writes }
    }
}

/// Per-call sink that persists one `llm_usage` row per LLM invocation.
#[derive(Debug)]
pub struct UsageRepoCallRecorder {
    sink: LlmUsageSink,
    tenant_id: String,
    user_id: String,
    conversation_id: Option<String>,
    turn_id: ConversationTurnId,
    call_type: &'static str,
}

impl UsageRepoCallRecorder {
    /// Build a recorder scoped to a single turn, writing through `sink`.
    #[must_use]
    pub fn new(
        sink: LlmUsageSink,
        tenant_id: String,
        user_id: String,
        conversation_id: Option<String>,
        turn_id: ConversationTurnId,
        call_type: &'static str,
    ) -> Self {
        Self {
            sink,
            tenant_id,
            user_id,
            conversation_id,
            turn_id,
            call_type,
        }
    }
}

impl LlmCallRecorder for UsageRepoCallRecorder {
    fn record(&self, record: LlmCallRecord) {
        let llm_usage = Arc::clone(&self.sink.repo);
        let tenant_id = self.tenant_id.clone();
        let user_id = self.user_id.clone();
        let conversation_id = self.conversation_id.clone();
        let turn_id = self.turn_id;
        let base_call_type = self.call_type;
        let call_sequence = record.call_sequence;
        let tenant_id_for_cost = self.tenant_id.clone();
        let write = async move {
            // Reasoning tokens are consumed and billed, and every provider that
            // reports them separately excludes them from `completion_tokens`, so
            // leaving them out made `total_tokens` smaller than the tokens the
            // turn actually spent.
            //
            // The tradeoff, recorded because it is not free: on a reasoning
            // model this no longer matches a provider dashboard that reports
            // `totalTokens = input + output`. The one ACP payload captured
            // 2026-08-27 had `thoughtTokens: 0`, so it could not settle whether
            // upstream counts them in its own total. Cost is unaffected either
            // way — `cost_from_pricing` charges reasoning at the output rate
            // from its own field, not from this sum.
            let total_tokens =
                record.prompt_tokens + record.completion_tokens + record.reasoning_tokens;
            let counts = TokenCounts::new(record.prompt_tokens, record.completion_tokens)
                .with_cache(record.cached_tokens, record.cached_write_tokens)
                .with_cache_write_1h(record.cached_write_1h_tokens)
                .with_reasoning(record.reasoning_tokens);
            let cost_usd = GLOBAL_PRICING_REGISTRY.calculate_cost(
                Some(tenant_id_for_cost.as_str()),
                &record.provider,
                &record.model,
                &counts,
            );
            // The line carries the counts its cost was priced from, the cache
            // split included, and the turn it belongs to, so a turn's spend is
            // one query on `turn_id` rather than a timestamp join against the
            // tool-loop lines and a back-solve of the cache split from
            // `cost_usd` against the price card (carnet#1194, carnet#1190).
            info!(
                target: "notify",
                event = "embacle.cost_usd",
                user_id = %user_id,
                tenant_id = %tenant_id,
                provider = %record.provider,
                model = %record.model,
                call_type = base_call_type,
                // Absent rather than "" when the call belongs to no
                // conversation, so a query on the field never matches a blank.
                conversation_id = conversation_id.as_deref(),
                turn_id = %turn_id,
                call_sequence = call_sequence,
                prompt_tokens = record.prompt_tokens,
                completion_tokens = record.completion_tokens,
                cached_read_tokens = record.cached_tokens,
                cached_write_tokens = record.cached_write_tokens,
                cached_write_1h_tokens = record.cached_write_1h_tokens,
                reasoning_tokens = record.reasoning_tokens,
                cost_usd = cost_usd,
                "llm call cost"
            );
            // A real LLM call that accounts for nothing is the silent failure
            // this warning exists to surface: the row lands priced at zero and,
            // with no `_estimated` suffix, reads downstream as a measured zero
            // rather than a missing measurement. It never fails the turn —
            // losing an accounting row beats failing the athlete — but it must
            // not pass unseen the way it did for 163 rows in 2026.
            if record.is_unaccounted() {
                warn!(
                    provider = %record.provider,
                    model = %record.model,
                    call_type = base_call_type,
                    success = record.success,
                    latency_ms = record.latency_ms,
                    "llm_usage row accounts for nothing: provider reported no usage and no text \
                     reached the estimator, so this call is recorded as free"
                );
            }
            let call_type_owned = if record.token_counts_estimated {
                format!("{base_call_type}_estimated")
            } else {
                base_call_type.to_owned()
            };
            let tool_calls_count = i64::try_from(record.tools_called.len()).unwrap_or(i64::MAX);
            let tools_called_json =
                serde_json::to_string(&record.tools_called).unwrap_or_else(|_| "[]".to_owned());
            let params = InsertLlmUsage {
                tenant_id: &tenant_id,
                user_id: &user_id,
                conversation_id: conversation_id.as_deref(),
                turn_id,
                provider: &record.provider,
                model: &record.model,
                prompt_tokens: record.prompt_tokens,
                completion_tokens: record.completion_tokens,
                total_tokens,
                cached_tokens: record.cached_tokens,
                cached_write_tokens: record.cached_write_tokens,
                cached_write_1h_tokens: record.cached_write_1h_tokens,
                reasoning_tokens: record.reasoning_tokens,
                call_type: &call_type_owned,
                tool_calls_count,
                tools_called: &tools_called_json,
                execution_time_ms: Some(record.latency_ms),
                cost_usd,
                call_sequence,
            };
            if let Err(e) = llm_usage.insert_llm_usage(&params).await {
                warn!("Failed to record per-LLM-call usage: {e}");
            }
        };
        self.sink.writes.spawn(async move {
            if AssertUnwindSafe(write).catch_unwind().await.is_err() {
                error!(
                    call_type = base_call_type,
                    "per-LLM-call usage write panicked; its llm_usage row is lost"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt::Debug;
    use std::sync::{Mutex, PoisonError};

    use pierre_test_support::db::create_test_db;
    use tracing::callsite::rebuild_interest_cache;
    use tracing::field::{Field, Visit};
    use tracing::subscriber::set_default;
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::Registry;

    use super::*;
    use crate::pricing::cost_for_record;

    /// The fields of every `llm call cost` event logged while installed.
    #[derive(Clone, Default)]
    struct CostLines(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

    struct FieldText<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for FieldText<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl<S: Subscriber> Layer<S> for CostLines {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldText(&mut fields));
            if fields.get("message").map(String::as_str) == Some("llm call cost") {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(fields);
            }
        }
    }

    /// The cost line names the turn, the call and the cache split it was
    /// priced from, so a turn's spend is one query on `turn_id` with no
    /// timestamp join and no back-solve of the split (carnet#1190, #1194).
    #[tokio::test]
    async fn the_cost_line_carries_the_turn_and_the_cache_split() {
        let db = create_test_db().await.expect("test db"); // Safe: test assertion
        let writes = TaskTracker::new();
        let sink = LlmUsageSink::new(Arc::clone(&db.repositories().llm_usage), writes.clone());
        let turn_id = ConversationTurnId::new();
        let recorder = UsageRepoCallRecorder::new(
            sink,
            "tenant-1".to_owned(),
            "user-1".to_owned(),
            Some("conv-1".to_owned()),
            turn_id,
            "chat",
        );
        let lines = CostLines::default();
        let _guard = set_default(Registry::default().with(lines.clone()));
        rebuild_interest_cache();

        recorder.record(LlmCallRecord {
            provider: "anthropic".to_owned(),
            model: "claude-opus-5-5".to_owned(),
            prompt_tokens: 70_000,
            completion_tokens: 1_000,
            cached_tokens: 62_000,
            cached_write_tokens: 7_000,
            cached_write_1h_tokens: 5_000,
            reasoning_tokens: 0,
            latency_ms: 900,
            success: true,
            call_sequence: Some(3),
            token_counts_estimated: false,
            tools_called: Vec::new(),
        });
        writes.close();
        writes.wait().await;

        let lines = lines
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let [line] = lines.as_slice() else {
            panic!("expected one cost line, got {lines:?}"); // Safe: test assertion
        };
        let field = |name: &str| line.get(name).map(String::as_str);
        let expected = [
            ("provider", "anthropic".to_owned()),
            ("model", "claude-opus-5-5".to_owned()),
            ("call_type", "chat".to_owned()),
            ("conversation_id", "conv-1".to_owned()),
            ("turn_id", turn_id.to_string()),
            ("call_sequence", "3".to_owned()),
            ("prompt_tokens", "70000".to_owned()),
            ("completion_tokens", "1000".to_owned()),
            ("cached_read_tokens", "62000".to_owned()),
            ("cached_write_tokens", "7000".to_owned()),
            ("cached_write_1h_tokens", "5000".to_owned()),
        ];
        for (name, value) in &expected {
            assert_eq!(
                field(name),
                Some(value.as_str()),
                "bug: the cost line carries {name}: {line:?}"
            );
        }
        // Opus 5.5 at $4/$20: 1,000 fresh ($0.004), 62,000 read at 0.05x
        // ($0.0124), 5,000 written for an hour at 2x ($0.04), 2,000 for five
        // minutes at 1.25x ($0.01), 1,000 out ($0.02).
        let logged: f64 = field("cost_usd")
            .and_then(|v| v.parse().ok())
            .expect("a numeric cost_usd"); // Safe: test assertion
        assert!(
            (logged - 0.0864).abs() < 1e-9,
            "bug: the logged cost must price the 1-hour and 5-minute writes apart: {line:?}"
        );

        // The row keeps the split, and pricing it again gives the logged cost.
        let rows = db
            .repositories()
            .llm_usage
            .find_llm_usage_by_turn_id(turn_id)
            .await
            .expect("the turn's rows"); // Safe: test assertion
        let [row] = rows.as_slice() else {
            panic!("expected one row, got {rows:?}"); // Safe: test assertion
        };
        assert_eq!(
            row.cached_write_1h_tokens, 5_000,
            "bug: the stored row must keep the 1-hour write split"
        );
        assert!(
            (cost_for_record(row) - logged).abs() < 1e-9,
            "bug: repricing the row must give the logged cost: {row:?}"
        );
        assert!(
            (row.cost_usd - logged).abs() < 1e-9,
            "bug: the stored cost must equal the logged one: {row:?}"
        );
    }

    /// A call outside any conversation logs no `conversation_id` at all,
    /// rather than an empty string a query would match.
    #[tokio::test]
    async fn a_call_outside_a_conversation_logs_no_conversation_id() {
        let db = create_test_db().await.expect("test db"); // Safe: test assertion
        let writes = TaskTracker::new();
        let sink = LlmUsageSink::new(Arc::clone(&db.repositories().llm_usage), writes.clone());
        let recorder = UsageRepoCallRecorder::new(
            sink,
            "tenant-1".to_owned(),
            "user-1".to_owned(),
            None,
            ConversationTurnId::new(),
            "chat",
        );
        let lines = CostLines::default();
        let _guard = set_default(Registry::default().with(lines.clone()));
        rebuild_interest_cache();
        recorder.record(LlmCallRecord {
            provider: "anthropic".to_owned(),
            model: "claude-opus-5-5".to_owned(),
            prompt_tokens: 100,
            completion_tokens: 10,
            cached_tokens: 0,
            cached_write_tokens: 0,
            cached_write_1h_tokens: 0,
            reasoning_tokens: 0,
            latency_ms: 10,
            success: true,
            call_sequence: None,
            token_counts_estimated: false,
            tools_called: Vec::new(),
        });
        writes.close();
        writes.wait().await;
        let lines = lines.0.lock().unwrap_or_else(PoisonError::into_inner);
        let [line] = lines.as_slice() else {
            panic!("expected one cost line, got {lines:?}"); // Safe: test assertion
        };
        assert!(
            !line.contains_key("conversation_id"),
            "bug: a call outside any conversation must log no conversation_id: {line:?}"
        );
    }
}
