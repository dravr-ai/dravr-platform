// ABOUTME: Outbound retry logic with exponential backoff and dead-letter queue
// ABOUTME: Computes next retry state from attempt count using configured delay schedule
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, TimeDelta, Utc};
use pierre_core::models::messaging::{MAX_RETRY_ATTEMPTS, RETRY_DELAYS_SECS};

/// Decision for the next retry action
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Schedule another retry at the given time
    Retry {
        /// Next retry timestamp
        next_retry_at: DateTime<Utc>,
        /// Updated status string (e.g., "retrying:2")
        status: String,
    },
    /// All retries exhausted, move to dead-letter queue
    DeadLetter,
}

/// Update to apply to an outbound queue entry after a delivery attempt
#[derive(Debug, Clone)]
pub struct RetryUpdate {
    /// New attempt count
    pub attempt_count: i32,
    /// Decision: retry or dead-letter
    pub decision: RetryDecision,
}

/// Compute the next retry state for a failed delivery attempt
///
/// Uses the delay schedule from `RETRY_DELAYS_SECS` (1s, 5s, 30s).
/// After `MAX_RETRY_ATTEMPTS` (3) failures, returns `DeadLetter`.
#[must_use]
pub fn compute_retry_update(current_attempt: i32) -> RetryUpdate {
    let next_attempt = current_attempt + 1;

    if next_attempt > MAX_RETRY_ATTEMPTS {
        return RetryUpdate {
            attempt_count: next_attempt,
            decision: RetryDecision::DeadLetter,
        };
    }

    // Index into delay schedule (0-based), clamp to last entry
    let idx = usize::try_from(next_attempt - 1).unwrap_or(0);
    let delay_secs = RETRY_DELAYS_SECS[idx.min(RETRY_DELAYS_SECS.len() - 1)];
    let next_retry_at = Utc::now() + TimeDelta::seconds(i64::try_from(delay_secs).unwrap_or(1));

    RetryUpdate {
        attempt_count: next_attempt,
        decision: RetryDecision::Retry {
            next_retry_at,
            status: format!("retrying:{next_attempt}"),
        },
    }
}
