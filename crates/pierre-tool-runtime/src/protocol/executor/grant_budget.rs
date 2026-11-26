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
use dravr_tronc::mcp::schema::ToolResponse;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_auth::oauth2_server::grant_standing::GrantStandings;
use pierre_core::constants::time_constants;
use pierre_providers::ai_scope::{self, ServedRows};
use pierre_services::usage_counter::{current_counter, increment_counter, LIFETIME_COUNTER_PREFIX};
use pierre_tools_core::ToolResult;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use tracing::{info, warn};

use super::subject::Parties;
use super::UniversalExecutor;
use crate::conversions::tool_result_to_response;
use crate::protocol::types::{UniversalResponse, GRANT_BLOCK};

tokio::task_local! {
    /// The single-use grant the enclosing top-level call consumed, in scope
    /// over that call's body: a dispatch nested in it runs under the grant
    /// the call already spent, and is neither refused nor claims it again.
    static CONSUMED_BY_THIS_CALL: String;
}

/// Whether `ceiling`'s grant is the single-use grant the enclosing call
/// consumed ([`CONSUMED_BY_THIS_CALL`]).
fn consumed_by_this_call(ceiling: &GrantCeiling) -> bool {
    CONSUMED_BY_THIS_CALL
        .try_with(|id| *id == ceiling.client_grant_id)
        .unwrap_or(false)
}

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
    /// The refusal of a call under `ceiling` whose single-use grant was
    /// consumed, or whose grant spent its total rows, or its rows or raw
    /// rows today; `None` when the call may run, outside a ceiling, and on a
    /// call carrying no tenant. A counter that cannot be read refuses the
    /// call, as a quota that cannot be read does.
    pub(super) async fn row_budget_refusal(
        &self,
        tool_name: &str,
        parties: Parties,
        ceiling: Option<&GrantCeiling>,
    ) -> Option<UniversalResponse> {
        let ceiling = ceiling?;
        let tenant = parties.requester_tenant?.to_string();
        let user = parties.requester.to_string();
        // A dispatch nested in the call that consumed the grant is that call.
        let nested = consumed_by_this_call(ceiling);
        if ceiling.single_use && !nested && !self.single_use_live(ceiling, &user, &tenant).await {
            info!(
                tool_name,
                client_grant_id = %ceiling.client_grant_id,
                "tool call refused: its single-use grant was already consumed"
            );
            return Some(consumed(tool_name));
        }
        if !nested {
            if let Some(refusal) = self
                .child_hour_refusal(tool_name, ceiling, &user, &tenant)
                .await
            {
                return Some(refusal);
            }
        }
        if let Some(refusal) = self
            .total_budget_refusal(tool_name, ceiling, &user, &tenant)
            .await
        {
            return Some(refusal);
        }
        self.daily_budget_refusal(tool_name, ceiling, &user, &tenant)
            .await
    }

    /// The refusal of a call under step-up child `ceiling` past its requests an hour.
    ///
    /// A one-off question's 30 (owl §3.1); `None` under a parent, whose hour
    /// the auth middleware meters on the token's grant.
    ///
    /// The call is counted before it is admitted, in one increment, so two
    /// calls arriving together cannot both take the last request. The hour
    /// is the clock's (`grant_child_requests:<id>:<hour since the epoch>`),
    /// a fixed window, where the parent's slides. A counter that cannot be
    /// written refuses.
    async fn child_hour_refusal(
        &self,
        tool_name: &str,
        ceiling: &GrantCeiling,
        user: &str,
        tenant: &str,
    ) -> Option<UniversalResponse> {
        ceiling.step_up.as_ref()?;
        let limit = ceiling.budgets.requests_per_hour;
        let now = Timestamp::now();
        let hour_secs = i64::from(time_constants::SECONDS_PER_HOUR);
        let hour = now.epoch_seconds().div_euclid(hour_secs);
        let counter = format!("grant_child_requests:{}:{hour}", ceiling.client_grant_id);
        let repo = self.resources.repos().usage_counters.as_ref();
        let over = increment_counter(repo, tenant, user, &counter, 1)
            .await
            .map_or_else(
                |e| {
                    warn!(
                        tool_name,
                        "a step-up grant's hourly requests could not be counted; refusing: {e}"
                    );
                    true
                },
                |made| made > i64::from(limit),
            );
        over.then(|| {
            info!(
                tool_name,
                client_grant_id = %ceiling.client_grant_id,
                "tool call refused: its step-up grant's requests this hour are spent"
            );
            let retry = hour_secs - now.epoch_seconds().rem_euclid(hour_secs);
            hour_exhausted(tool_name, retry)
        })
    }

    /// The refusal of a call under `ceiling` whose grant served every row it
    /// may over its life; `None` for a grant with no total budget. A counter
    /// that cannot be read refuses.
    async fn total_budget_refusal(
        &self,
        tool_name: &str,
        ceiling: &GrantCeiling,
        user: &str,
        tenant: &str,
    ) -> Option<UniversalResponse> {
        let limit = ceiling.budgets.total_rows?;
        let repo = self.resources.repos().usage_counters.as_ref();
        let total = current_counter(repo, tenant, user, &total_rows_counter(ceiling)).await;
        let spent_total = total.as_ref().map_or_else(
            |e| {
                warn!(
                    tool_name,
                    "a grant's total row budget could not be read; refusing: {e}"
                );
                true
            },
            |total| spent(*total, Some(limit)),
        );
        spent_total.then(|| {
            info!(
                tool_name,
                client_grant_id = %ceiling.client_grant_id,
                "tool call refused: its consent grant's total row budget is spent"
            );
            total_exhausted(tool_name)
        })
    }

    /// The refusal of a call under `ceiling` whose grant spent its rows or
    /// raw rows today. A counter that cannot be read refuses.
    async fn daily_budget_refusal(
        &self,
        tool_name: &str,
        ceiling: &GrantCeiling,
        user: &str,
        tenant: &str,
    ) -> Option<UniversalResponse> {
        let repo = self.resources.repos().usage_counters.as_ref();
        let budgets = ceiling.budgets;
        let rows = current_counter(repo, tenant, user, &rows_counter(ceiling)).await;
        let raw_rows = current_counter(repo, tenant, user, &raw_rows_counter(ceiling)).await;
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

    /// Whether single-use grant `ceiling` is still live: the standing a
    /// token was admitted on may be cached past the call that consumed it.
    /// A grant that cannot be read is not.
    async fn single_use_live(&self, ceiling: &GrantCeiling, user: &str, tenant: &str) -> bool {
        match self
            .resources
            .repos()
            .oauth2_server
            .get_client_grant(&ceiling.client_grant_id, user, tenant)
            .await
        {
            Ok(grant) => grant.is_some_and(|grant| grant.is_live_at(Timestamp::now())),
            Err(e) => {
                warn!("a single-use grant could not be read; refusing: {e}");
                false
            }
        }
    }

    /// Run `body`, a call's, under `ceiling`, counting the rows its readers
    /// serve, and levy them on its grant's counters ([`Self::levy_rows`]).
    pub(super) async fn levied<F: Future>(
        &self,
        parties: Parties,
        ceiling: Option<&GrantCeiling>,
        body: F,
    ) -> F::Output {
        let (output, served) = ai_scope::counting_rows(body).await;
        self.levy_rows(served, parties, ceiling).await;
        output
    }

    /// Run `body`, a call's, once `ceiling`'s single-use grant is consumed
    /// for it.
    ///
    /// Past every gate (owl §3.1: consumed by the first call that reads data
    /// under it): the refusal when it is not this call's to read; `body`
    /// when the call may read, and under every other grant.
    ///
    /// The one statement that ends the grant decides, so of two calls
    /// arriving together exactly one reads; the other, and every later one,
    /// is refused `grant_consumed`. A call admitted here that then reads
    /// nothing (its arguments refused) has still used the grant: the grant
    /// is spent before the read, never after, so no read outlives it. A
    /// failed write refuses the call. The grant is consumed once per
    /// top-level call: `body` runs with it marked consumed by this call, so
    /// a dispatch nested in it neither claims it again nor is refused.
    pub(super) async fn single_use_claimed<F>(
        &self,
        tool_name: &str,
        parties: Parties,
        ceiling: Option<&GrantCeiling>,
        body: F,
    ) -> ToolResponse
    where
        F: Future<Output = ToolResponse>,
    {
        let Some(ceiling) = ceiling.filter(|ceiling| ceiling.single_use) else {
            return body.await;
        };
        if consumed_by_this_call(ceiling) {
            return body.await;
        }
        let claimed = match parties.requester_tenant {
            Some(tenant) => self
                .resources
                .repos()
                .oauth2_server
                .consume_child_grant(
                    &ceiling.client_grant_id,
                    &parties.requester.to_string(),
                    &tenant.to_string(),
                )
                .await
                .unwrap_or_else(|e| {
                    warn!(
                        client_grant_id = %ceiling.client_grant_id,
                        "a single-use grant could not be consumed; refusing: {e}"
                    );
                    false
                }),
            None => false,
        };
        if claimed {
            info!(
                tool_name,
                client_grant_id = %ceiling.client_grant_id,
                "single-use grant consumed by the call reading under it"
            );
            self.forget_standing(parties).await;
            return CONSUMED_BY_THIS_CALL
                .scope(ceiling.client_grant_id.clone(), body)
                .await;
        }
        info!(
            tool_name,
            client_grant_id = %ceiling.client_grant_id,
            "tool call refused: its single-use grant was already consumed"
        );
        let refusal = consumed(tool_name);
        tool_result_to_response(Ok(ToolResult::error(json!({
            "error_code": GRANT_CONSUMED,
            "reason": GRANT_CONSUMED,
            "error": refusal.error,
        }))))
    }

    /// Drop the cached standing of the calling client's consent, which still
    /// holds the child this call consumed: introspection and the next
    /// token's admission read it, and would echo a consumed child live until
    /// the entry expired.
    async fn forget_standing(&self, parties: Parties) {
        let (Some(tenant), Some(client_id)) = (
            parties.requester_tenant,
            self.credential
                .as_ref()
                .and_then(|credential| credential.client_id.as_deref()),
        ) else {
            return;
        };
        let standings = GrantStandings::new(
            self.resources.repos().oauth2_server.clone(),
            self.resources.cache().clone(),
        );
        let (user, tenant, client_id) =
            (parties.requester, tenant.to_string(), client_id.to_owned());
        // Its own task, awaited: the cache's future is deep enough that
        // carrying it inside every dispatch's overflows the `Send` check of
        // the callers that spawn a dispatch.
        let forgotten =
            tokio::spawn(async move { standings.forget(user, &tenant, &client_id).await });
        if let Err(e) = forgotten.await {
            warn!("a consumed grant's cached standing could not be dropped: {e}");
        }
    }

    /// Add what a call under a ceiling served to its grant's counters today.
    /// Best effort: a failed write is logged, and the next call is admitted
    /// on the counts the store holds.
    async fn levy_rows(
        &self,
        served: ServedRows,
        parties: Parties,
        ceiling: Option<&GrantCeiling>,
    ) {
        let (Some(ceiling), Some(tenant)) = (ceiling, parties.requester_tenant) else {
            return;
        };
        let tenant = tenant.to_string();
        let user = parties.requester.to_string();
        let repo = self.resources.repos().usage_counters.as_ref();
        let total = ceiling
            .budgets
            .total_rows
            .map(|_| (total_rows_counter(ceiling), served.rows));
        let levies = [
            (rows_counter(ceiling), served.rows),
            (raw_rows_counter(ceiling), served.raw_rows),
        ]
        .into_iter()
        .chain(total);
        for (counter, amount) in levies {
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

/// The in-band refusal of `tool_name`: its step-up grant made every request
/// it may this hour, the next one `retry_after_secs` away.
fn hour_exhausted(tool_name: &str, retry_after_secs: i64) -> UniversalResponse {
    let mut refusal = budget_exhausted(tool_name, retry_after_secs);
    refusal.error = Some(format!(
        "'{tool_name}' was not run: the one-off access the athlete gave this application has \
         made all the requests it may this hour. Retry in {retry_after_secs} seconds."
    ));
    refusal
}

/// The `error_code` of a call refused because its single-use grant was
/// consumed (owl §7.4 `grant_consumed`).
const GRANT_CONSUMED: &str = "grant_consumed";

/// The in-band refusal of `tool_name`: its single-use grant was consumed by
/// an earlier call.
fn consumed(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": GRANT_CONSUMED,
            "reason": GRANT_CONSUMED,
        })),
        error: Some(format!(
            "'{tool_name}' was not run: the one-off access the athlete gave this application was \
             used by an earlier call. Ask the athlete to approve a new question."
        )),
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
            "'{tool_name}' was not run: the one-off access the athlete gave this application has \
             served all the data it may. Ask the athlete to approve a new question."
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
