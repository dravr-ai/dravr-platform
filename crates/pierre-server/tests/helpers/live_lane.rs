// ABOUTME: Lane mechanics the live-model lanes share: the contremaitre overlay wait, per-turn usage rows, the turn wait
// ABOUTME: Generic over what a lane grades; used by the incident corpus and the injection eval, and their offline guards
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

//! How a live lane waits and what it reads back from a turn.
//!
//! Both live lanes run a real model on a test server: they wait for its
//! contremaitre sync before the first turn, wait each turn out past the
//! server's own turn watchdog, and read what the turn's model calls asked for
//! from the `llm_usage` rows the turn wrote. Shared across test binaries; a
//! binary that includes the helpers may use none of it.

use pierre_config::timeouts::ServerTimeouts;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::messaging_ingress::turn_watchdog;
use std::collections::{BTreeSet, HashSet};
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Poll interval while waiting for the overlay sync or a turn's usage rows.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How many polls a turn's `llm_usage` rows get to settle after the reply.
const USAGE_SETTLE_POLLS: usize = 10;

/// How long a pass waits for its server's first contremaitre sync.
///
/// A full sync from the GitHub store takes ~50 s (`tool_sync.rs`); the bound
/// leaves room for a slow API without letting a dead sync stall the job.
const CONTREMAITRE_SYNC_DEADLINE: Duration = Duration::from_secs(240);

/// How far past the server's turn watchdog a lane keeps waiting: room for the
/// turn to start behind its acknowledgement and for the watchdog's closing
/// notice to land. The real-Slack lane waits the same two minutes past it
/// (`REPLY_WAIT_SECS` in `messaging_eval_real_slack_test.rs`).
const TURN_WAIT_MARGIN: Duration = Duration::from_secs(120);

// -----------------------------------------------------------------------
// Waiting
// -----------------------------------------------------------------------

/// Hold the pass until the server has applied its contremaitre overlays, when
/// the lane was pointed at a contremaitre ref (`CONTREMAITRE_REPO`).
///
/// The server's first sync runs in the background, off the boot path, and the
/// first turn goes out about a second after boot — so without this wait a run
/// meant to grade a ref's tool descriptions grades the compiled-in ones, and
/// with no agent prompt in the `PromptRegistry` every coach falls back to its
/// `agents.system_prompt` column, which is not what an athlete is served. The
/// tool overlays land in one swap (`update_all`) after the system prompts, so
/// a non-empty tool-description registry means both are live. A sync that
/// never lands is a lane fault, not a draw: the pass stops rather than grade
/// a surface it was not asked to.
pub(crate) async fn await_contremaitre_overlay(resources: &Arc<ServerContext>) {
    if !env::var("CONTREMAITRE_REPO").is_ok_and(|v| !v.is_empty()) {
        return;
    }
    let started = Instant::now();
    let registry = &resources.mcp.tool_description_registry;
    while registry.count() == 0 {
        assert!(
            started.elapsed() < CONTREMAITRE_SYNC_DEADLINE,
            "CONTREMAITRE_REPO is set but no tool-description overlay landed within {}s — \
             the pass would grade the compiled-in descriptions, not the ref it names",
            CONTREMAITRE_SYNC_DEADLINE.as_secs()
        );
        sleep(POLL_INTERVAL).await;
    }
    println!(
        "contremaitre overlay live after {:.1}s: {} tool descriptions, tool_discipline_shared {} B (ref {})",
        started.elapsed().as_secs_f64(),
        registry.count(),
        resources
            .mcp
            .prompt_registry
            .tool_discipline_shared_prompt()
            .len(),
        env::var("CONTREMAITRE_BRANCH").unwrap_or_default(),
    );
}

/// How long a lane waits for one turn before recording it as an infra error.
///
/// Above the server's own ceiling on a turn ([`turn_watchdog`],
/// `MESSAGING_TURN_WATCHDOG_SECS`), read live so an operator who moves the
/// ceiling moves this with it. Every turn has therefore ended — with its
/// reply, or with the watchdog's notice — before the lane posts the next one.
/// A wait below it lets a slow turn keep running past the point the lane
/// gave up on it, and its reply then lands after the next question, where the
/// lane reads it as that turn's answer and grades it against the wrong
/// question: an infra delay reported as a coach failure, or a coach failure
/// hidden behind the previous turn's good answer.
///
/// No single tier's cap is the right bound once the runtime fallback is
/// armed: a `copilot_sdk` turn that times out at
/// `EMBACLE_SDK_PROMPT_TIMEOUT_SECS` re-runs on the `claude_code` tier, so
/// one healthy turn can run past every per-tier cap. The watchdog is the one
/// bound every tier and every transport runs under.
pub(crate) fn turn_timeout() -> Duration {
    turn_timeout_under(&lane_timeouts())
}

/// [`turn_timeout`] for a server under `timeouts`.
pub(crate) fn turn_timeout_under(timeouts: &ServerTimeouts) -> Duration {
    turn_watchdog(timeouts).saturating_add(TURN_WAIT_MARGIN)
}

/// The time bounds a lane's servers run under: the operator's, read from the
/// environment as a deployed server reads them at startup, so the lane waits
/// out the same watchdog its servers apply.
pub(crate) fn lane_timeouts() -> ServerTimeouts {
    ServerTimeouts::from_env()
}

// -----------------------------------------------------------------------
// Usage rows
// -----------------------------------------------------------------------

/// One `llm_usage` row: `(id, tools_called, prompt_tokens, provider)`.
type UsageRow = (String, String, i64, String);

/// Every `llm_usage` row of the tenant, oldest first.
async fn usage_rows(resources: &Arc<ServerContext>, tenant: TenantId) -> Vec<UsageRow> {
    const SQL: &str = "SELECT id, tools_called, CAST(prompt_tokens AS BIGINT), provider \
                       FROM llm_usage WHERE tenant_id = $1 ORDER BY created_at";
    let tenant = tenant.to_string();
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_as(SQL)
            .bind(&tenant)
            .fetch_all(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query_as(SQL)
            .bind(&tenant)
            .fetch_all(pg.pool())
            .await
            .unwrap(),
    }
}

/// The ids of the tenant's `llm_usage` rows so far: what a turn's rows are
/// told apart from.
pub(crate) async fn usage_ids(resources: &Arc<ServerContext>, tenant: TenantId) -> HashSet<String> {
    usage_rows(resources, tenant)
        .await
        .into_iter()
        .map(|(id, ..)| id)
        .collect()
}

/// What the model did during one turn, from the `llm_usage` rows it added.
pub(crate) struct TurnUsage {
    /// Every tool a call of the turn asked for, in call order — what the model
    /// asked for, whether it then ran, failed or was refused.
    pub tools: Vec<String>,
    /// How many model calls the turn made.
    pub calls: usize,
    /// Prompt tokens over those calls: what deferral is meant to shrink.
    pub prompt_tokens: i64,
    /// The providers that served them: the tier that answered.
    pub served_by: BTreeSet<String>,
}

/// What the turn's model calls were, from the rows added since `before`.
///
/// The rows are written on a drained background task, so the reply can land
/// first: read until two reads a poll apart agree.
pub(crate) async fn turn_usage(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    before: &HashSet<String>,
) -> TurnUsage {
    let mut last = 0;
    for _ in 0..USAGE_SETTLE_POLLS {
        sleep(POLL_INTERVAL).await;
        let count = usage_rows(resources, tenant)
            .await
            .iter()
            .filter(|(id, ..)| !before.contains(id))
            .count();
        if count == last && count > 0 {
            break;
        }
        last = count;
    }
    let added: Vec<UsageRow> = usage_rows(resources, tenant)
        .await
        .into_iter()
        .filter(|(id, ..)| !before.contains(id))
        .collect();
    TurnUsage {
        tools: added
            .iter()
            .flat_map(|(_, tools, ..)| {
                serde_json::from_str::<Vec<String>>(tools).unwrap_or_default()
            })
            .collect(),
        calls: added.len(),
        prompt_tokens: added.iter().map(|(_, _, tokens, _)| tokens).sum(),
        served_by: added
            .into_iter()
            .map(|(_, _, _, provider)| provider)
            .collect(),
    }
}
