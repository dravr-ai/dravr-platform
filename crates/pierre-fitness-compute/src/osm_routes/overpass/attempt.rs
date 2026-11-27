// ABOUTME: One request to one Overpass instance and why it gave no usable answer: status,
// ABOUTME: timeout, unreadable body, or a query the server stopped short; and the log's outcome line
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::OverpassClient;
use pierre_core::constants::project::user_agent;
use reqwest::header::USER_AGENT;
use reqwest::StatusCode;
use serde::Deserialize;
use std::fmt;
use std::time::Duration;
use tracing::debug;

/// How an Overpass `remark` starts when the server stopped the query before
/// it finished — its `[timeout:N]` ran out, or its memory did — and answered
/// 200 with the elements found so far. A `runtime remark` is informational
/// and starts otherwise.
pub(super) const RUNTIME_ERROR_REMARK: &str = "runtime error";

/// Why one request to one Overpass instance produced no usable answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AttemptFailure {
    /// 502/503/504: the instance is up and its dispatcher is saturated.
    /// Overpass answers this within seconds, so it earns one retry.
    Busy(StatusCode),
    /// 429: the instance asks us to back off.
    RateLimited(StatusCode),
    /// Any other unsuccessful status.
    Rejected(StatusCode),
    /// No complete answer within the attempt's timeout.
    TimedOut(Duration),
    /// The connection failed or broke off.
    Network(String),
    /// A 200 whose body is not an Overpass JSON response — free instances
    /// answer an overload with an HTML error page.
    Unparseable(String),
    /// A 200 whose `remark` says the server stopped the query (it timed out
    /// or ran out of memory): its elements are only what it found before
    /// stopping, so they are not the area's routes.
    CutShort(String),
}

impl AttemptFailure {
    /// Short label for the structured log field.
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::Busy(_) => "busy",
            Self::RateLimited(_) => "rate_limited",
            Self::Rejected(_) => "rejected",
            Self::TimedOut(_) => "timeout",
            Self::Network(_) => "network",
            Self::Unparseable(_) => "unparseable",
            Self::CutShort(_) => "cut_short",
        }
    }

    /// Whether the instance gets a second attempt in the same walk.
    pub(super) const fn earns_retry(&self) -> bool {
        matches!(self, Self::Busy(_))
    }

    /// Whether the failure says the instance is saturated, rather than that
    /// the query, the answer or the connection went wrong.
    pub(super) const fn is_overload(&self) -> bool {
        matches!(
            self,
            Self::Busy(_) | Self::RateLimited(_) | Self::TimedOut(_) | Self::CutShort(_)
        )
    }

    /// Whether the instance moves to the back of the order for a while: it
    /// cost a full timeout (its own, for a query it cut short), is
    /// unreachable, or asked us to back off.
    pub(super) const fn cools_down(&self) -> bool {
        matches!(
            self,
            Self::RateLimited(_) | Self::TimedOut(_) | Self::Network(_) | Self::CutShort(_)
        )
    }
}

impl fmt::Display for AttemptFailure {
    /// The reason as the route-search log records it: no HTML, no URL, no
    /// stack.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy(status) => write!(f, "busy (HTTP {})", status.as_u16()),
            Self::RateLimited(status) => write!(f, "rate limited (HTTP {})", status.as_u16()),
            Self::Rejected(status) => write!(f, "refused (HTTP {})", status.as_u16()),
            Self::TimedOut(after) => write!(f, "no answer within {}s", after.as_secs()),
            Self::Network(_) => f.write_str("connection failed"),
            Self::Unparseable(_) => f.write_str("unreadable response"),
            Self::CutShort(_) => f.write_str("stopped the query before it finished"),
        }
    }
}

/// The `remark` of an Overpass body whose query the server stopped before it
/// finished, or `None` for a complete answer.
///
/// Only a body that names a remark is parsed a second time: a complete
/// answer carries none.
pub(super) fn cut_short_remark(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Remarked {
        remark: Option<String>,
    }
    if !body.contains("\"remark\"") {
        return None;
    }
    let remark = serde_json::from_str::<Remarked>(body).ok()?.remark?;
    remark
        .trim_start()
        .starts_with(RUNTIME_ERROR_REMARK)
        .then_some(remark)
}

/// Every attempt's reason and the instances never reached, one per host, for
/// the route-search log.
pub(super) fn walk_outcomes(failures: &[(String, AttemptFailure)], untried: &[String]) -> String {
    failures
        .iter()
        .map(|(host, failure)| format!("{host}: {failure}"))
        .chain(
            untried
                .iter()
                .map(|host| format!("{host}: not tried, time budget spent")),
        )
        .collect::<Vec<_>>()
        .join("; ")
}

impl OverpassClient {
    /// One request to one instance, returning its raw body.
    pub(super) async fn attempt(
        &self,
        endpoint: &str,
        query: &str,
        timeout: Duration,
    ) -> Result<String, AttemptFailure> {
        debug!(endpoint, ?timeout, "Querying Overpass instance");

        let response = self
            .client
            .post(endpoint)
            .header(USER_AGENT, user_agent())
            .timeout(timeout)
            .form(&[("data", query)])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AttemptFailure::TimedOut(timeout)
                } else {
                    AttemptFailure::Network(e.to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(match status {
                StatusCode::TOO_MANY_REQUESTS => AttemptFailure::RateLimited(status),
                StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT => AttemptFailure::Busy(status),
                _ => AttemptFailure::Rejected(status),
            });
        }

        let body = response.text().await.map_err(|e| {
            if e.is_timeout() {
                AttemptFailure::TimedOut(timeout)
            } else {
                AttemptFailure::Network(e.to_string())
            }
        })?;
        if let Some(remark) = cut_short_remark(&body) {
            return Err(AttemptFailure::CutShort(remark));
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logged_outcomes_name_every_host_and_why() {
        let failures = vec![
            (
                "overpass-api.de".to_owned(),
                AttemptFailure::Busy(StatusCode::BAD_GATEWAY),
            ),
            (
                "overpass.private.coffee".to_owned(),
                AttemptFailure::TimedOut(Duration::from_secs(5)),
            ),
            (
                "maps.mail.ru".to_owned(),
                AttemptFailure::CutShort("runtime error: Query timed out".to_owned()),
            ),
        ];
        assert_eq!(
            walk_outcomes(&failures, &["spare.example".to_owned()]),
            "overpass-api.de: busy (HTTP 502); overpass.private.coffee: no answer within 5s; \
             maps.mail.ru: stopped the query before it finished; \
             spare.example: not tried, time budget spent",
            "bug: the log loses a host's outcome"
        );
    }
}
