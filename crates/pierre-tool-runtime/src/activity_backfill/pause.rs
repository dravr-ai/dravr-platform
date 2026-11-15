// ABOUTME: When a walk a rate limit paused may try again: the refusing window's end, else a lease from now
// ABOUTME: Shared by the history walk and the detail walk, so neither is re-claimed each sweep at a call's cost

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppError;

use super::lease_ms;

/// When a rate-limit refusal lets the walk try again (unix ms): the end of
/// the refusing window, which a budget refusal names; a lease from now when
/// it names none. Never the next sweep: a walk released due at once is
/// claimed again within a minute, and each claim spends one more call the
/// refusal would refuse.
pub(super) fn retry_at_ms(refusal: &AppError) -> i64 {
    let wait_ms = refusal.retry_after_secs().map_or_else(lease_ms, |secs| {
        i64::try_from(secs)
            .unwrap_or(i64::MAX)
            .saturating_mul(1_000)
    });
    Utc::now().timestamp_millis().saturating_add(wait_ms)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::errors::{AppError, ErrorCode};

    use super::{lease_ms, retry_at_ms};

    #[test]
    fn a_refusal_naming_no_wait_pauses_a_lease_not_a_sweep() {
        let before = Utc::now().timestamp_millis();
        let until = retry_at_ms(&AppError::new(ErrorCode::ExternalRateLimited, "throttled"));
        let after = Utc::now().timestamp_millis();
        assert!(
            (before + lease_ms()..=after + lease_ms()).contains(&until),
            "bug: a refusal naming no wait leaves the walk due at the next sweep, a call a pass: {until}"
        );
    }

    #[test]
    fn a_refusal_naming_its_wait_pauses_until_it_ends() {
        let before = Utc::now().timestamp_millis();
        let until = retry_at_ms(
            &AppError::new(ErrorCode::ExternalRateLimited, "budget spent").with_retry_after(120),
        );
        let after = Utc::now().timestamp_millis();
        assert!(
            (before + 120_000..=after + 120_000).contains(&until),
            "bug: a refusal's named wait is not the pause: {until}"
        );
    }
}
