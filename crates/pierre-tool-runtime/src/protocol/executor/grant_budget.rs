// ABOUTME: A consent grant's daily row budgets at dispatch: checked before each call, levied with what the call served
// ABOUTME: Counted in usage_counters per UTC day; an exhausted budget is an in-band budget_exhausted with retry_after_secs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The daily row budgets of a consent grant (plan slice 3 item 8, owl PDPP
//! §3.1 and §9.1).
//!
//! The readers below dispatch count what they serve under the grant's
//! ceiling ([`ai_scope::counting_rows`]): rows, and the time-series samples
//! among them. Every call made under a ceiling passes here, the one dispatch
//! path `/mcp`, A2A and the tasks worker share: it is admitted on the day's
//! counts before it, so it may overshoot by its own size once, and its own
//! counts are then added. The counters live in `usage_counters`, per tenant
//! and user, under `grant_rows:<client_grant_id>` and
//! `grant_raw_rows:<client_grant_id>`, in the UTC day. A grant budgeted in
//! rows over its whole life (a one-off question, owl §3.1 `total_rows`) counts
//! them under `lifetime_grant_rows:<client_grant_id>`, which never resets. A
//! budget the grant has no value for is not checked.

use dravr_tronc::dates::Timestamp;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_core::constants::time_constants;
use pierre_providers::ai_scope::{self, ServedRows};
use pierre_services::usage_counter::{
    current_counter, increment_counter, LIFETIME_COUNTER_PREFIX,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use tracing::{info, warn};

use super::subject::Parties;
use super::UniversalExecutor;
use crate::protocol::types::{UniversalResponse, GRANT_BLOCK};

/// The `error_code` of a call refused because its grant's day is spent.
const BUDGET_EXHAUSTED: &str = "budget_exhausted";

/// The counter of a grant's rows served today.
fn rows_counter(ceiling: &GrantCeiling) -> String {
    format!("grant_rows:{}", ceiling.client_grant_id)
}

/// The counter of a grant's time-series samples served today.
fn raw_rows_counter(ceiling: &GrantCeiling) -> String {
    format!("grant_raw_rows:{}", ceiling.client_grant_id)
}

/// The counter of a grant's rows served over its whole life.
fn total_rows_counter(ceiling: &GrantCeiling) -> String {
    format!(
        "{LIFETIME_COUNTER_PREFIX}grant_rows:{}",
        ceiling.client_grant_id
    )
}

/// Whether `served` has reached `limit`; a budget with no limit never has.
fn spent(served: i64, limit: Option<u32>) -> bool {
    limit.is_some_and(|limit| served >= i64::from(limit))
}

/// Seconds from `now` to the next UTC midnight, when the counters reset.
fn seconds_to_utc_midnight(now: Timestamp) -> i64 {
    let day = i64::from(time_constants::SECONDS_PER_DAY);
    day - now.epoch_seconds().rem_euclid(day)
}

impl UniversalExecutor {
    /// The refusal of a call under a ceiling whose grant spent its rows or
    /// its raw rows today; `None` when the call may run, outside a ceiling,
    /// and on a call carrying no tenant. A counter that cannot be read
    /// refuses the call, as a quota that cannot be read does.
    pub(super) async fn row_budget_refusal(
        &self,
        tool_name: &str,
        parties: Parties,
    ) -> Option<UniversalResponse> {
        let ceiling = self.ceiling.as_ref()?;
        let tenant = parties.requester_tenant?.to_string();
        let user = parties.requester.to_string();
        let repo = self.resources.repos().usage_counters.as_ref();
        let budgets = ceiling.budgets;
        let rows = current_counter(repo, &tenant, &user, &rows_counter(ceiling)).await;
        let raw_rows = current_counter(repo, &tenant, &user, &raw_rows_counter(ceiling)).await;
        let total = match budgets.total_rows {
            Some(_) => current_counter(repo, &tenant, &user, &total_rows_counter(ceiling)).await,
            None => Ok(0),
        };
        let total_spent = match &total {
            Ok(total) => spent(*total, budgets.total_rows),
            Err(_) => true,
        };
        if total_spent {
            if let Err(e) = &total {
                warn!(
                    tool_name,
                    "a grant's total row budget could not be read; refusing: {e}"
                );
            }
            info!(
                tool_name,
                client_grant_id = %ceiling.client_grant_id,
                "tool call refused: its consent grant's total row budget is spent"
            );
            return Some(total_exhausted(tool_name));
        }
        let day_spent = match (rows, raw_rows) {
            (Ok(rows), Ok(raw_rows)) => {
                spent(rows, budgets.rows_per_day) || spent(raw_rows, budgets.raw_rows_per_day)
            }
            (Err(e), _) | (_, Err(e)) => {
                warn!(
                    tool_name,
                    "a grant's row budget could not be read; refusing: {e}"
                );
                true
            }
        };
        day_spent.then(|| {
            info!(
                tool_name,
                client_grant_id = %ceiling.client_grant_id,
                "tool call refused: its consent grant's daily row budget is spent"
            );
            budget_exhausted(tool_name, seconds_to_utc_midnight(Timestamp::now()))
        })
    }

    /// Run `body`, a call's, counting the rows its readers serve, and levy
    /// them on its grant's counters ([`Self::levy_rows`]).
    pub(super) async fn levied<F: Future>(&self, parties: Parties, body: F) -> F::Output {
        let (output, served) = ai_scope::counting_rows(body).await;
        self.levy_rows(served, parties).await;
        output
    }

    /// Add what a call under a ceiling served to its grant's counters today.
    /// Best effort: a failed write is logged, and the next call is admitted
    /// on the counts the store holds.
    async fn levy_rows(&self, served: ServedRows, parties: Parties) {
        let (Some(ceiling), Some(tenant)) = (self.ceiling.as_ref(), parties.requester_tenant)
        else {
            return;
        };
        let tenant = tenant.to_string();
        let user = parties.requester.to_string();
        let repo = self.resources.repos().usage_counters.as_ref();
        let total = ceiling
            .budgets
            .total_rows
            .map(|_| (total_rows_counter(ceiling), served.rows));
        for (counter, amount) in [
            (rows_counter(ceiling), served.rows),
            (raw_rows_counter(ceiling), served.raw_rows),
        ]
        .into_iter()
        .chain(total)
        {
            if amount == 0 {
                continue;
            }
            let amount = i64::try_from(amount).unwrap_or(i64::MAX);
            if let Err(e) = increment_counter(repo, &tenant, &user, &counter, amount).await {
                warn!(counter, "failed to levy a grant's served rows: {e}");
            }
        }
    }
}

/// The in-band refusal of `tool_name`: its grant's day is spent until the
/// next UTC midnight, `retry_after_secs` away (owl PDPP §9.1).
fn budget_exhausted(tool_name: &str, retry_after_secs: i64) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": BUDGET_EXHAUSTED,
            "reason": BUDGET_EXHAUSTED,
            "retry_after_secs": retry_after_secs,
        })),
        error: Some(format!(
            "'{tool_name}' was not run: the access the athlete gave this application has served \
             all the data it may today. Retry in {retry_after_secs} seconds."
        )),
        // Stamped, so a transport can pass the refusal's data on.
        metadata: Some(HashMap::from([(
            "blocked_reason".to_owned(),
            Value::from(GRANT_BLOCK),
        )])),
    }
}

/// The `error_code` of a call refused because its grant served every row it
/// may over its life (owl §3.1: `health_budget_exhausted`).
const TOTAL_BUDGET_EXHAUSTED: &str = "health_budget_exhausted";

/// The in-band refusal of `tool_name`: its grant served all the rows it may
/// ever serve; no retry helps, only a new grant.
fn total_exhausted(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": TOTAL_BUDGET_EXHAUSTED,
            "reason": BUDGET_EXHAUSTED,
        })),
        error: Some(format!(
            "'{tool_name}' was not run: the one-off access the athlete gave this application has              served all the data it may. Ask the athlete to approve a new question."
        )),
        metadata: Some(HashMap::from([(
            "blocked_reason".to_owned(),
            Value::from(GRANT_BLOCK),
        )])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_retry_waits_for_the_next_utc_midnight() {
        let late = Timestamp::parse_from_rfc3339("2026-11-22T23:59:00Z").unwrap();
        assert_eq!(
            seconds_to_utc_midnight(late),
            60,
            "bug: a minute before midnight is not 60 s from the reset"
        );
        let midnight = Timestamp::parse_from_rfc3339("2026-11-22T00:00:00Z").unwrap();
        assert_eq!(
            seconds_to_utc_midnight(midnight),
            86_400,
            "bug: at midnight the reset is not a whole day away"
        );
    }

    #[test]
    fn an_exhausted_budget_says_when_to_retry() {
        let refused = budget_exhausted("get_activities", 60);
        assert!(!refused.success, "bug: a spent budget read as a success");
        let result = refused.result.unwrap();
        assert_eq!(
            result["error_code"], BUDGET_EXHAUSTED,
            "bug: the refusal does not name budget_exhausted"
        );
        assert_eq!(
            result["retry_after_secs"], 60,
            "bug: the refusal does not say when to retry"
        );
    }

    #[test]
    fn a_budget_without_a_limit_is_never_spent() {
        assert!(!spent(i64::MAX, None), "bug: an unlimited budget was spent");
        assert!(spent(500, Some(500)), "bug: a reached limit was not spent");
        assert!(!spent(499, Some(500)), "bug: a budget spent one row early");
    }

    #[test]
    fn a_spent_total_names_health_budget_exhausted_and_no_retry() {
        let result = total_exhausted("get_health_records").result.unwrap();
        assert_eq!(
            result["error_code"], TOTAL_BUDGET_EXHAUSTED,
            "bug: the total refusal does not name health_budget_exhausted"
        );
        assert!(
            result.get("retry_after_secs").is_none(),
            "bug: a spent total offered a retry"
        );
    }
}
