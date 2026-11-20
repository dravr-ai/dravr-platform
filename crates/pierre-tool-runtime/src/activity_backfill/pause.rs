// ABOUTME: When a walk a rate limit paused may try again: the refusing window's end, else a lease from now
// ABOUTME: Shared by the history walk and the detail walk, so neither is re-claimed each sweep at a call's cost

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_providers::request_budget::{refused_before_send, session_read_disabled};

use super::lease_ms;

/// When a rate-limit refusal lets the walk try again (unix ms): the end of
/// the refusing window, which a budget refusal names; a lease from now when
/// it names none. Never the next sweep: a walk released due at once is
/// claimed again within a minute, and each claim spends one more call the
/// refusal would refuse.
pub(super) fn retry_at_ms(refusal: &AppError) -> i64 {
    retry_at_after(refusal.retry_after_secs())
}

/// The moment (unix ms) `retry_after_secs` from now; a lease from now when
/// no wait is named.
pub(super) fn retry_at_after(retry_after_secs: Option<u64>) -> i64 {
    let wait_ms = retry_after_secs.map_or_else(lease_ms, |secs| {
        i64::try_from(secs)
            .unwrap_or(i64::MAX)
            .saturating_mul(1_000)
    });
    Utc::now().timestamp_millis().saturating_add(wait_ms)
}

/// When a walk whose read `error` refused may try again (unix ms), when it
/// is a rate limit: a request budget's, a session-read cap's (carnet#1661) or
/// the provider's own. Waiting for it spends none of the walk's attempts.
/// `None` for any other failure, and for a session-read cap of `0`
/// ([`session_read_disabled`]): no window ends that refusal, so the walk
/// fails and its attempts end it rather than waiting forever.
pub(super) fn paused_by(error: &AppError) -> Option<i64> {
    (error.code == ErrorCode::ExternalRateLimited && !session_read_disabled(error))
        .then(|| retry_at_ms(error))
}

/// Whether a run a request budget refused (a session-read cap included)
/// waits for the refusing window to end, its attempt returned. Not a
/// session-read cap of `0` ([`session_read_disabled`]): the run fails, and
/// its attempts end it.
pub(super) fn waits_out(error: &AppError) -> bool {
    refused_before_send(error) && !session_read_disabled(error)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::errors::{AppError, ErrorCode};
    use pierre_providers::request_budget::refused_before_send;

    use super::{lease_ms, paused_by, retry_at_ms, waits_out};

    #[test]
    fn a_rate_limited_read_pauses_and_any_other_failure_does_not() {
        let refusal = AppError::new(
            ErrorCode::ExternalRateLimited,
            "Garmin has been read as often as Dravr allows for now",
        )
        .with_retry_after(900);
        let before = Utc::now().timestamp_millis();
        let until = paused_by(&refusal)
            .unwrap_or_else(|| panic!("bug: a rate-limited walk read fails instead of pausing"));
        assert!(
            until >= before + 900_000,
            "bug: the pause ends before the refusing window: {until}"
        );
        assert!(
            !refused_before_send(&refusal),
            "the pause does not depend on who refused: a provider 429 waits too"
        );
        for other in [
            AppError::new(ErrorCode::ExternalServiceError, "scraper 502"),
            AppError::new(ErrorCode::ProviderAuthRequired, "session expired"),
        ] {
            assert_eq!(
                paused_by(&other),
                None,
                "bug: a {:?} read pauses the walk instead of failing it",
                other.code
            );
        }
    }

    /// A session-read cap of 0 refuses every read until an admin raises it:
    /// a walk or a run it refused fails, and its attempts end it, instead of
    /// waiting forever for a window that never lets a read through.
    #[test]
    fn a_zero_session_read_cap_fails_instead_of_pausing() {
        let refusal = |disabled: bool| {
            let mut details = serde_json::Map::new();
            details.insert("refused_before_send".to_owned(), serde_json::json!(true));
            details.insert(
                "session_read_cap".to_owned(),
                serde_json::json!("background"),
            );
            if disabled {
                details.insert("session_read_disabled".to_owned(), serde_json::json!(true));
            }
            let mut error = AppError::new(ErrorCode::ExternalRateLimited, "Garmin read as allowed");
            error.details = Some(Box::new(serde_json::Value::Object(details)));
            error.with_retry_after(900)
        };
        let spent = refusal(false);
        assert!(
            paused_by(&spent).is_some() && waits_out(&spent),
            "bug: a spent session-read window does not wait for its end"
        );
        let disabled = refusal(true);
        assert_eq!(
            paused_by(&disabled),
            None,
            "bug: a walk a cap of 0 refused pauses, and is re-claimed forever"
        );
        assert!(
            !waits_out(&disabled),
            "bug: a run a cap of 0 refused waits, and is re-claimed forever"
        );
    }

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
