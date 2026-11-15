// ABOUTME: get_segment_history's memo of Strava refusing an athlete its live segment listing
// ABOUTME: In process and bounded: a 402/403 keeps that athlete off the listing for a day

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which athletes Strava refused the live listing of a segment's efforts,
//! and for how long the tool answers them from stored details instead.

use std::num::NonZeroUsize;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration as StdDuration, Instant};

use lru::LruCache;
use pierre_core::errors::AppError;
use pierre_providers::core::is_segment_listing_refused;
use uuid::Uuid;

/// How long a failure of the live listing ([`provider_refused`]) keeps the
/// athlete off it.
const REFUSAL_MEMO_TTL: StdDuration = StdDuration::from_hours(24);

/// Athletes the refusal memo holds at most; the least recently refused goes
/// first.
const REFUSAL_MEMO_ATHLETES: NonZeroUsize = match NonZeroUsize::new(10_000) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// When each athlete's live listing through a backend was last refused by
/// the provider ([`provider_refused`]), by
/// `(athlete, backend)`: in process, bounded, read against
/// [`REFUSAL_MEMO_TTL`].
static REFUSAL_MEMO: LazyLock<Mutex<LruCache<(Uuid, String), Instant>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(REFUSAL_MEMO_ATHLETES)));

/// Whether Strava refused `key`'s live listing within [`REFUSAL_MEMO_TTL`]
/// of `now`.
pub(super) fn refused_lately(key: &(Uuid, String), now: Instant) -> bool {
    REFUSAL_MEMO
        .lock()
        .ok()
        .and_then(|mut memo| memo.get(key).copied())
        .is_some_and(|at| now.saturating_duration_since(at) < REFUSAL_MEMO_TTL)
}

/// Remember that Strava refused `key`'s live listing at `now`.
pub(super) fn remember_refusal(key: (Uuid, String), now: Instant) {
    if let Ok(mut memo) = REFUSAL_MEMO.lock() {
        memo.put(key, now);
    }
}

/// Whether `error` keeps the athlete off the listing for a day: only the
/// provider refusing this athlete the listing, which needs a subscription
/// (Strava's `402` or `403`, [`is_segment_listing_refused`]). It stands for
/// every segment, so it is worth a day's wait.
///
/// Nothing else is: a `404` names a segment id the caller mistyped, a `5xx`,
/// a request that could not be sent and an open circuit breaker are this
/// request's, a gate that refused before any request (this transport, the
/// athlete's consent), a spent budget or rate limit and a dead grant are
/// read again on their own terms, and an id that is not a number is the
/// call's. None says the athlete cannot list.
pub(super) fn provider_refused(error: &AppError) -> bool {
    is_segment_listing_refused(error)
}

#[cfg(test)]
mod tests {
    use pierre_core::constants::oauth::providers::STRAVA;
    use pierre_providers::core::segment_listing_refused;
    use pierre_providers::delegation::athlete_off_roster;

    use super::*;

    #[test]
    fn a_refusal_strava_answered_is_remembered_for_a_day() {
        let key = (Uuid::new_v4(), STRAVA.to_owned());
        let at = Instant::now();
        assert!(!refused_lately(&key, at), "bug: a fresh athlete is refused");
        remember_refusal(key.clone(), at);
        assert!(
            refused_lately(&key, at + StdDuration::from_hours(23)),
            "bug: a refusal is forgotten within the day"
        );
        assert!(
            !refused_lately(&key, at + REFUSAL_MEMO_TTL),
            "bug: a refusal outlives its day"
        );
    }

    #[test]
    fn only_strava_refusing_the_athlete_the_listing_is_remembered() {
        for status in [402, 403] {
            let refused = segment_listing_refused(STRAVA, status);
            assert!(
                provider_refused(&refused),
                "bug: Strava refusing a non-subscriber ({status}) is not remembered"
            );
        }
        for passing in [
            // What the request itself answered, as `api_error` words it: a
            // mistyped id, a failing Strava; a request never sent; an open
            // breaker.
            AppError::external_service(
                STRAVA,
                "Provider strava API error: 404 - strava API request failed with status 404 Not Found",
            ),
            AppError::external_service(
                STRAVA,
                "Provider strava API error: 500 - strava API request failed with status 500 Internal Server Error",
            ),
            AppError::external_service(STRAVA, "Failed to send request: connection refused"),
            AppError::external_service(
                "Strava",
                "Circuit breaker open for provider strava, retry after 30s",
            ),
            // Gates that refused before any request, and the call's own.
            AppError::unavailable_over_transport("this data is not available over this interface"),
            AppError::not_found("strava data is withheld from AI"),
            AppError::provider_auth_required(STRAVA),
            AppError::invalid_input("a Strava segment id is a number"),
            // A provider's permission refusal that is not the listing's.
            athlete_off_roster("Strava"),
        ] {
            assert!(
                !provider_refused(&passing),
                "bug: an error that says nothing of the athlete keeps them off the listing: {passing:?}"
            );
        }
    }
}
