// ABOUTME: Strava's own active streak, read off the signed-in athlete's profile and carried on the stats
// ABOUTME: A figure the provider reports, never one Dravr derives; a profile that cannot be read leaves it absent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's active streak, as Strava states it on their dashboard
//! (sciotte 2.1 reads it with the profile).
//!
//! It is Strava's own figure: nothing here derives it from activities, and
//! a streak Strava did not report is `None`, never zero days. One profile
//! read answers it, and its failure never fails the stats it rides on.

use dravr_sciotte::models::{ActiveStreak as SciotteStreak, AthleteProfile};
use pierre_core::models::ActiveStreak;
use tracing::warn;

use super::{SciotteProvider, SciotteTarget};
use crate::errors::AppResult;

/// Strava's streak as the platform's.
pub(super) fn convert(streak: SciotteStreak) -> ActiveStreak {
    ActiveStreak {
        length: streak.length,
        status: streak.status,
        valid_activity_count: streak.valid_activity_count,
    }
}

impl SciotteProvider {
    /// The signed-in Strava account's own streak, or `None` when the backend
    /// is not Strava, the provider reads a coached athlete rather than the
    /// account itself, Strava reported none, or the profile read failed
    /// (logged).
    pub(super) async fn read_active_streak(&self) -> Option<ActiveStreak> {
        let target = SciotteTarget::from_backend_name(self.provider_name);
        if !matches!(target, SciotteTarget::Strava) || self.subject.is_some() {
            return None;
        }
        match self.read_profile().await {
            Ok(profile) => profile.active_streak.map(convert),
            Err(e) => {
                warn!(provider = self.provider_name, error = %e, "the profile read for the streak failed; the stats carry none");
                None
            }
        }
    }

    /// The signed-in account's profile, on the dedicated read service.
    async fn read_profile(&self) -> AppResult<AthleteProfile> {
        let session = self.session_snapshot().await?;
        let remote = self.remote_for(&session)?;
        self.read_through(remote, &session, || remote.get_athlete(&session.session_id))
            .await
    }
}

#[cfg(test)]
mod tests {
    use dravr_sciotte::models::ActiveStreak as SciotteStreak;

    use super::convert;

    #[test]
    fn strava_reports_cross_as_they_stand() {
        let out = convert(SciotteStreak {
            length: 21,
            status: "Ongoing".to_owned(),
            valid_activity_count: 640,
        });
        assert_eq!(
            (out.length, out.status.as_str(), out.valid_activity_count),
            (21, "Ongoing", 640),
            "bug: Strava's own figures were recomputed or reworded"
        );
    }
}
