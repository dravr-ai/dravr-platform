// ABOUTME: The detail walk's day windows, and the background share it reserves from every other background request
// ABOUTME: Strava's background 250 a day split exactly: 125 the detail walk's own, the rest the history walk's

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How the windows of a request in the background are sized at a provider
//! with detail-walk windows (Strava).
//!
//! The detail walk ([`in_detail_walk`](super::in_detail_walk)) is counted in
//! two day windows of its own: every athlete's walk in the app's, half the
//! background share of the app's day, and each athlete's in theirs.
//!
//! The app's walk window is a reservation, not only a cap (JF's directive on
//! carnet#1585): every other background request, the walk of an athlete's
//! history above all, is admitted only while the app's day, whoever spent
//! it, holds fewer than the background share less what the detail walk has
//! not yet spent of its window. The detail walk always finds the rest of its
//! 125 of Strava's 1,000 under the background share, so the history walk can
//! never take them, whatever sized the day.
//!
//! The reservation holds for the whole day: a slice the detail walk has not
//! spent is never lent to the history walk, since a fixed day bucket cannot
//! know at seven in the morning whether the walk will want it by night. The
//! history walk runs slower by the same amount, the cost the issue named.
//! Requests an athlete is waiting on still come first and count in the day:
//! they shrink what every other background request may spend before they
//! shrink the detail walk's reservation, and both walks stop once the app's
//! day holds its background share.

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use uuid::Uuid;

use super::{
    budget_period, detail_walk_athlete_counter_key, detail_walk_burst_counter_key,
    detail_walk_counter_key, Budget, Holder, ProviderRateLimiter, Window, BACKGROUND_SHARE_PERCENT,
    FIFTEEN_MINUTES, ONE_DAY, PLATFORM_SCOPE, SESSION_WALK_BURST_CALLS,
};

/// The share, in percent, of the background share of an app's day reserved
/// for every athlete's detail walk together: half, 125 of Strava's standard
/// 1,000 (JF's call on carnet#1483, made a reservation on carnet#1585), so
/// the walk of an athlete's history keeps the other half.
const DETAIL_WALK_PERCENT_OF_BACKGROUND: u32 = 50;

/// The day windows of the background at a provider with detail-walk windows,
/// for one holder: counted under the holder's day window's key, the
/// athlete's held to the app's, and the app's and the rest of the background
/// share adding up to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DetailWalkSizes {
    day_key: String,
    athlete_calls: u32,
    app_calls: u32,
    rest_calls: u32,
}

/// A day window of `max_calls`, sized already, so taken whole.
fn whole_day(key: String, max_calls: u32) -> Window {
    Window {
        key,
        budget: Budget {
            max_calls,
            window: ONE_DAY,
        },
        share_percent: 100,
    }
}

impl ProviderRateLimiter {
    /// `windows`, the holder's windows at `provider`, preceded by the two
    /// daily windows of `user_id`'s detail walk when `provider` has them:
    /// the athlete's, then the app's.
    ///
    /// The app's walk window is half the background share of the holder's
    /// day as `windows` carries it, so whatever sized the day — the
    /// provider's, the configured one, a pool's or a tenant app's registered
    /// limit — sizes it; the athlete's is [`super::DETAIL_WALK_DAILY_CALLS`], never
    /// more than the app's. Both are taken whole: their sizes are already
    /// the walk's share. They come first, so a walk whose day is spent is
    /// refused until that day ends without touching the windows the reads an
    /// athlete is waiting on count in.
    ///
    /// A holder with no daily window has no day to take a share of, and its
    /// walk is held to its background share alone.
    ///
    /// A [`Holder::Session`] is counted in the walk windows alone: they are
    /// the app's, the ones the grant's walk spends, but its reads never touch
    /// the app's own API windows, which a session does not use. Its burst
    /// window comes first.
    pub(super) fn with_detail_walk_windows(
        &self,
        provider: &str,
        holder: &Holder,
        user_id: Uuid,
        windows: Vec<Window>,
    ) -> Vec<Window> {
        let session = matches!(holder, Holder::Session { .. });
        let Some(sizes) = self.detail_walk_sizes(provider, &windows) else {
            return if session { Vec::new() } else { windows };
        };
        let mut walk = Vec::new();
        // A signed-in session's walk is also spread over the day: its burst
        // window comes first, so a refusal by it takes nothing from the
        // day's.
        if session {
            walk.push(Window {
                key: detail_walk_burst_counter_key(&sizes.day_key, user_id),
                budget: Budget {
                    max_calls: SESSION_WALK_BURST_CALLS,
                    window: FIFTEEN_MINUTES,
                },
                share_percent: 100,
            });
        }
        walk.extend([
            whole_day(
                detail_walk_athlete_counter_key(&sizes.day_key, user_id),
                sizes.athlete_calls,
            ),
            whole_day(detail_walk_counter_key(&sizes.day_key), sizes.app_calls),
        ]);
        if !session {
            walk.extend(windows);
        }
        walk
    }

    /// `windows`, the holder's windows at `provider` for a background
    /// request outside a detail walk, with the app's day held to what the
    /// detail walk's reservation leaves when `provider` has detail-walk
    /// windows: the rest of the background share, plus what the detail walk
    /// has already spent of its window today (the day holding `now`).
    ///
    /// Live reads, the history walk and the detail walk all count in the
    /// app's day, so the room left under the background share for the
    /// detail walk is never less than what it has yet to spend. The detail
    /// walk's spend is read just before the take: a walk call landing after
    /// the read only narrows this window, and the one race that widens it, a
    /// walk call counted that a later window then refuses and gives back,
    /// lasts that call's round trip and widens it by that call alone.
    ///
    /// # Errors
    /// A database error when the detail walk's window cannot be read.
    pub(super) async fn with_detail_walk_reserved(
        &self,
        provider: &str,
        windows: Vec<Window>,
        now: Timestamp,
    ) -> AppResult<Vec<Window>> {
        let Some(sizes) = self.detail_walk_sizes(provider, &windows) else {
            return Ok(windows);
        };
        let walk_spent = self
            .counters
            .get_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                &detail_walk_counter_key(&sizes.day_key),
                &budget_period(now, ONE_DAY),
            )
            .await?
            .value
            .clamp(0, i64::from(sizes.app_calls));
        let room = sizes
            .rest_calls
            .saturating_add(u32::try_from(walk_spent).unwrap_or(sizes.app_calls));
        Ok(windows
            .into_iter()
            .map(|window| {
                if window.budget.window == ONE_DAY && window.key == sizes.day_key {
                    whole_day(window.key, room)
                } else {
                    window
                }
            })
            .collect())
    }

    /// The day windows of the background at `provider` for a holder whose
    /// windows are `windows`; `None` when `provider` has no walk windows or
    /// the holder no daily window.
    fn detail_walk_sizes(&self, provider: &str, windows: &[Window]) -> Option<DetailWalkSizes> {
        let athlete_calls = self.detail_walks.get(provider).map(|calls| *calls)?;
        let day = windows
            .iter()
            .find(|window| window.budget.window == ONE_DAY)?;
        let background_calls =
            u32::try_from(day.budget.allowance(BACKGROUND_SHARE_PERCENT)).unwrap_or(u32::MAX);
        let background_day = Budget {
            max_calls: background_calls,
            window: ONE_DAY,
        };
        let app_calls = u32::try_from(background_day.allowance(DETAIL_WALK_PERCENT_OF_BACKGROUND))
            .unwrap_or(u32::MAX);
        Some(DetailWalkSizes {
            day_key: day.key.clone(),
            athlete_calls: athlete_calls.min(app_calls),
            app_calls,
            rest_calls: background_calls.saturating_sub(app_calls),
        })
    }

    /// The calls a day one athlete's detail walk at `provider` may make for
    /// `holder`, as [`Self::with_detail_walk_windows`] sizes its window.
    pub(super) async fn detail_walk_daily_calls(
        &self,
        provider: &str,
        holder: &Holder,
    ) -> Option<u32> {
        let windows = self.windows_for(provider, holder, 100).await;
        self.detail_walk_sizes(provider, &windows)
            .map(|sizes| sizes.athlete_calls)
    }
}
