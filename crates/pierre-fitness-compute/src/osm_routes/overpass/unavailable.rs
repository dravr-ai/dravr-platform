// ABOUTME: What a walk that found no Overpass instance answering tells the athlete, and the
// ABOUTME: short hold that answers the same instances' next searches without asking them again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::AttemptFailure;
use pierre_core::errors::{AppError, ErrorCode};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// What a caller reads when no instance answered and every one said it was
/// saturated: busy, rate limited, hung, or stopping the query. In words an
/// athlete understands — no host, status code or timing; those go to the
/// structured log the route-search alert counts
/// (`infra/environments/dev/route_search_monitoring.tf`).
pub(super) const ROUTE_SEARCH_OVERLOADED: &str =
    "The public OpenStreetMap route search is overloaded and not answering right now, so no \
     map route could be listed. Trying again later may work; do not suggest routes from memory.";

/// What a caller reads when no instance answered and at least one failed in
/// a way that does not say it was saturated — a refused query, an answer
/// that could not be read, a connection that failed. The fault may be ours,
/// so the athlete is not told the public service is overloaded.
pub(super) const ROUTE_SEARCH_FAILED: &str =
    "The OpenStreetMap route search did not return a usable answer right now, so no map route \
     could be listed. Trying again later may work; do not suggest routes from memory.";

/// The `event` field of the one warning a walk that found no instance
/// answering logs: the key the route-search metric filters on.
pub(super) const ROUTE_SEARCH_UNAVAILABLE_EVENT: &str = "route_search_unavailable";

/// The `event` field of the line a search answered from a recent failed walk
/// logs. Distinct from [`ROUTE_SEARCH_UNAVAILABLE_EVENT`], so the alert counts
/// walks, not an agent's retries of one.
pub(super) const ROUTE_SEARCH_HELD_EVENT: &str = "route_search_unavailable_held";

/// How long a walk that found no instance answering is the answer for the
/// same instances, without asking any of them again.
///
/// An agent told "trying again later may work" retries within seconds; each
/// retry would otherwise ask the saturated volunteer servers once more and
/// log one more unavailable search, so one athlete's turn could page by
/// itself. A minute is short enough that a recovered instance is asked
/// again soon. Only a walk that asked every instance is held: one the budget
/// never reached is asked first by the next walk.
pub(super) const OVERPASS_UNAVAILABLE_HOLD: Duration = Duration::from_secs(60);

/// Why a walk found no instance answering, as the athlete is told it and
/// the log's `cause` field records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WalkCause {
    /// Every instance asked said it was saturated.
    Overloaded,
    /// At least one failed otherwise: the fault may be ours.
    Failed,
}

impl WalkCause {
    pub(super) fn of(failures: &[(String, AttemptFailure)]) -> Self {
        if failures.iter().all(|(_, failure)| failure.is_overload()) {
            Self::Overloaded
        } else {
            Self::Failed
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Overloaded => "overloaded",
            Self::Failed => "failed",
        }
    }

    pub(super) fn error(self) -> AppError {
        let message = match self {
            Self::Overloaded => ROUTE_SEARCH_OVERLOADED,
            Self::Failed => ROUTE_SEARCH_FAILED,
        };
        AppError::new(ErrorCode::ExternalServiceUnavailable, message)
    }
}

/// A failed walk's answer for the instances it asked, until it ends.
#[derive(Debug, Clone, Copy)]
struct Hold {
    until: Instant,
    cause: WalkCause,
}

/// Process-wide record of the instance sets whose last walk found none
/// answering, keyed by the instances in configured order.
static UNAVAILABLE_HOLDS: LazyLock<RwLock<HashMap<Vec<String>, Hold>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Why the last walk over `endpoints` failed, while its hold lasts.
pub(super) fn held_unavailable(endpoints: &[String], now: Instant) -> Option<WalkCause> {
    let holds = UNAVAILABLE_HOLDS.read().ok()?;
    holds
        .get(endpoints)
        .and_then(|held| (held.until > now).then_some(held.cause))
}

/// Answer the next searches over `endpoints` with `cause` for `hold`.
pub(super) fn hold_unavailable(endpoints: &[String], cause: WalkCause, hold: Duration) {
    if let Ok(mut holds) = UNAVAILABLE_HOLDS.write() {
        let until = Instant::now() + hold;
        holds.insert(endpoints.to_vec(), Hold { until, cause });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn only_saturated_instances_make_the_overloaded_message() {
        let saturated = |failure: AttemptFailure| ("host".to_owned(), failure);
        let overloaded = [
            saturated(AttemptFailure::Busy(StatusCode::BAD_GATEWAY)),
            saturated(AttemptFailure::RateLimited(StatusCode::TOO_MANY_REQUESTS)),
            saturated(AttemptFailure::TimedOut(Duration::from_secs(5))),
            saturated(AttemptFailure::CutShort(
                "runtime error: timed out".to_owned(),
            )),
        ];
        assert_eq!(WalkCause::of(&overloaded), WalkCause::Overloaded);
        for ours in [
            AttemptFailure::Rejected(StatusCode::BAD_REQUEST),
            AttemptFailure::Network("dns error".to_owned()),
            AttemptFailure::Unparseable("expected value".to_owned()),
        ] {
            let mut failures = overloaded.to_vec();
            failures.push(saturated(ours.clone()));
            assert_eq!(
                WalkCause::of(&failures),
                WalkCause::Failed,
                "bug: {ours:?} may be our fault, never told as an overload"
            );
        }
        assert!(
            !ROUTE_SEARCH_FAILED.contains("overloaded"),
            "bug: a failure that may be ours is not called an overload"
        );
    }
}
