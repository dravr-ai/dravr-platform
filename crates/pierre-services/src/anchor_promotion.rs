// ABOUTME: The weekly promotion rule for the profile's resting heart rate, VO2max and weight (DECISION 5)
// ABOUTME: Fills only an empty or unvouched anchor from the athlete's own measurements; an athlete's or imported one stands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Anchor promotion (Athlete Data Consolidation §5, DECISION 5)
//!
//! The profile's resting heart rate, `VO2max` and weight are the anchors
//! stored training load and the heart-rate zones are scored against
//! ([`crate::effective_physiology`]). A watch measures the same three every
//! day; pointing the anchors at the latest reading would recut the zones and
//! every stored day's load each morning. So once a week the promotion copies
//! the athlete's measurement into an anchor, and only into one nobody vouches
//! for: empty, an estimate, or what an earlier promotion wrote
//! ([`AnchorProvenance::promotable`]). A number the athlete stated or one
//! imported from a system holding their own is never overwritten.
//!
//! Which measurement:
//! - **Resting heart rate:** the median of the morning readings of the
//!   athlete's last [`RESTING_HR_WINDOW_DAYS`] local days, one per day as the
//!   merged recovery read gives them, and only when at least
//!   [`RESTING_HR_MIN_MORNINGS`] mornings hold one. A median, so one restless
//!   night or a sensor glitch does not move the anchor.
//! - **`VO2max`:** the latest stored device value, the running one (the push
//!   stores no cycling value as `vo2_max`).
//! - **Weight:** the latest weigh-in.
//!
//! The rule decides which number fills the profile; the training science that
//! scores against it stays in dravr-cageux and pierre-fitness-compute.

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::civil_time::clock_date;
use pierre_core::errors::AppResult;
use pierre_core::models::{AnchorProvenance, AnchorStamp, TenantId, UserPhysiologicalProfile};
use pierre_core::transport::TransportPolicy;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::athlete_clock::athlete_zone;
use crate::stored_health::{self, Anchors, SourceFilter};

/// Local days whose morning resting heart rate the promotion takes the median
/// of.
const RESTING_HR_WINDOW_DAYS: i64 = 7;

/// Mornings inside the window that must hold a resting heart rate before the
/// median is promoted: fewer would let a single reading stand for the week.
const RESTING_HR_MIN_MORNINGS: usize = 3;

/// One anchor's measurement: its value, the day it stands for, and the
/// transport stamp of the rows read to find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured<T> {
    /// The value the anchor would take.
    pub value: T,
    /// The day it stands for.
    pub as_of: NaiveDate,
    /// What the rows read for this anchor alone allow (carnet#769): a profile
    /// is stamped with it only when this anchor is the one that moves.
    pub policy: TransportPolicy,
}

/// The measurement each promotable anchor would take; `None` where no
/// measurement qualifies.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Promotion {
    /// Resting heart rate, bpm, as of the newest morning in the window.
    pub resting_hr: Option<Measured<u16>>,
    /// `VO2max`, ml/kg/min, as of the day it was measured.
    pub vo2_max: Option<Measured<f64>>,
    /// Weight, kg, as of the day it was measured.
    pub weight: Option<Measured<f64>>,
}

impl Promotion {
    /// Whether no anchor has a measurement to take.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.resting_hr.is_none() && self.vo2_max.is_none() && self.weight.is_none()
    }

    /// The transport stamp a profile owes for the `moved` anchors: the
    /// strictest of the reads behind those anchors only, so rows read for an
    /// anchor that did not move never stamp the profile.
    #[must_use]
    pub fn policy_of(&self, moved: Anchors) -> TransportPolicy {
        TransportPolicy::strictest_of(
            [
                self.resting_hr
                    .filter(|_| moved.resting_hr)
                    .map(|m| m.policy),
                self.vo2_max.filter(|_| moved.vo2_max).map(|m| m.policy),
                self.weight.filter(|_| moved.weight).map(|m| m.policy),
            ]
            .into_iter()
            .flatten(),
        )
    }
}

/// The anchors of `profile` the promotion may fill or replace: every one when
/// no profile is stored.
#[must_use]
pub fn promotable_anchors(profile: Option<&UserPhysiologicalProfile>) -> Anchors {
    let Some(profile) = profile else {
        return Anchors {
            resting_hr: true,
            vo2_max: true,
            weight: true,
        };
    };
    let provenance = &profile.anchor_provenance;
    Anchors {
        resting_hr: AnchorProvenance::promotable(
            profile.resting_hr.is_some(),
            provenance.resting_hr.as_ref(),
        ),
        vo2_max: AnchorProvenance::promotable(
            profile.vo2_max.is_some(),
            provenance.vo2_max.as_ref(),
        ),
        weight: AnchorProvenance::promotable(profile.weight.is_some(), provenance.weight.as_ref()),
    }
}

/// The median of the morning resting heart rates, rounded to the nearest
/// beat; `None` below [`RESTING_HR_MIN_MORNINGS`] readings or for a value no
/// heart rate takes.
#[must_use]
fn median_resting_hr(mornings: &[u32]) -> Option<u16> {
    if mornings.len() < RESTING_HR_MIN_MORNINGS {
        return None;
    }
    let mut sorted = mornings.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        // Half up: the mean of two whole beats is a whole or a half beat.
        (sorted[middle - 1] + sorted[middle]).div_ceil(2)
    } else {
        sorted[middle]
    };
    u16::try_from(median).ok()
}

/// The athlete's measurement for each anchor `wanted` names, read under the
/// provider terms and gates in force like every wellness read.
///
/// Each anchor is read in a derivation of its own ([`ai_scope::derived`]), so
/// each carries the stamp of the rows behind it alone: a first-party-only
/// provider's mornings that promote no resting heart rate never stamp a
/// profile whose weight alone moved.
///
/// # Errors
///
/// Returns an error when the recovery or body rows, or under a gate the
/// athlete's data sources, cannot be read.
pub async fn promotion_measurements(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    tenant: TenantId,
    user_id: Uuid,
    wanted: Anchors,
) -> AppResult<Promotion> {
    let mut promotion = Promotion::default();
    if wanted.resting_hr {
        let (median, policy) =
            ai_scope::derived(week_median_resting_hr(repos, terms, tenant, user_id)).await;
        promotion.resting_hr = median?.map(|(value, as_of)| Measured {
            value,
            as_of,
            policy,
        });
    }
    if wanted.vo2_max {
        let (latest, policy) = ai_scope::derived(stored_health::latest_anchor_measurements(
            repos,
            terms,
            user_id,
            &tenant,
            Anchors {
                vo2_max: true,
                ..Anchors::default()
            },
        ))
        .await;
        promotion.vo2_max = latest?.vo2_max.map(|m| Measured {
            value: m.value,
            as_of: m.date,
            policy,
        });
    }
    if wanted.weight {
        let (latest, policy) = ai_scope::derived(stored_health::latest_anchor_measurements(
            repos,
            terms,
            user_id,
            &tenant,
            Anchors {
                weight: true,
                ..Anchors::default()
            },
        ))
        .await;
        promotion.weight = latest?.weight_kg.map(|m| Measured {
            value: m.value,
            as_of: m.date,
            policy,
        });
    }
    Ok(promotion)
}

/// The median of the athlete's morning resting heart rates over their last
/// [`RESTING_HR_WINDOW_DAYS`] local days, as of the newest of them.
async fn week_median_resting_hr(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    tenant: TenantId,
    user_id: Uuid,
) -> AppResult<Option<(u16, NaiveDate)>> {
    let now = Utc::now();
    let today = clock_date(now, athlete_zone(repos, user_id).await);
    let first_day = today - Duration::days(RESTING_HR_WINDOW_DAYS - 1);
    // The read bounds on the UTC date; a day of margin keeps every local
    // morning of the window inside it, and the dates are narrowed below.
    let mornings = stored_health::merged_recovery(
        repos,
        terms,
        user_id,
        &tenant,
        (now - Duration::days(RESTING_HR_WINDOW_DAYS + 1), now),
        SourceFilter::All,
        stored_health::as_stored,
    )
    .await?
    .records;
    let in_window: Vec<(u32, NaiveDate)> = mornings
        .iter()
        .filter(|morning| (first_day..=today).contains(&morning.record.date))
        .filter_map(|morning| {
            morning
                .record
                .resting_heart_rate
                .map(|bpm| (bpm, morning.record.date))
        })
        .collect();
    let readings: Vec<u32> = in_window.iter().map(|(bpm, _)| *bpm).collect();
    Ok(median_resting_hr(&readings).and_then(|median| {
        in_window
            .iter()
            .map(|(_, date)| *date)
            .max()
            .map(|newest| (median, newest))
    }))
}

/// Write `promotion` into each anchor of `profile` it may fill, stamped as
/// promoted. Returns the anchors that moved: one already holding the same
/// promoted value as of the same day is left as it is.
///
/// The rule is re-checked here, per anchor, against the profile it is applied
/// to, so a promotion read for an anchor that profile marks as the athlete's
/// is dropped.
pub fn apply_promotion(profile: &mut UserPhysiologicalProfile, promotion: &Promotion) -> Anchors {
    let promotable = promotable_anchors(Some(profile));
    let mut moved = Anchors::default();
    if let Some(m) = promotion.resting_hr.filter(|_| promotable.resting_hr) {
        moved.resting_hr = promote(
            &mut profile.resting_hr,
            &mut profile.anchor_provenance.resting_hr,
            m,
        );
    }
    if let Some(m) = promotion.vo2_max.filter(|_| promotable.vo2_max) {
        moved.vo2_max = promote(
            &mut profile.vo2_max,
            &mut profile.anchor_provenance.vo2_max,
            m,
        );
    }
    if let Some(m) = promotion.weight.filter(|_| promotable.weight) {
        moved.weight = promote(
            &mut profile.weight,
            &mut profile.anchor_provenance.weight,
            m,
        );
    }
    moved
}

/// Set one anchor to a promoted measurement; whether it moved.
///
/// A different value as of the day the stored promotion already stands for
/// is not applied: the table reads a value that changes under an unchanged
/// stamp as a writer that knows no origins (a revision before them), and
/// hands the anchor to the athlete. The next measurement, as of a later day,
/// refreshes it.
fn promote<T: PartialEq + Copy>(
    anchor: &mut Option<T>,
    stamp: &mut Option<AnchorStamp>,
    measured: Measured<T>,
) -> bool {
    let promoted = Some(AnchorStamp::promoted(measured.as_of));
    if *stamp == promoted {
        return false;
    }
    *anchor = Some(measured.value);
    *stamp = promoted;
    true
}

#[cfg(test)]
mod tests {
    use pierre_core::models::{AnchorOrigin, SportType};

    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap_or_default()
    }

    fn profile() -> UserPhysiologicalProfile {
        UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Run)
    }

    #[test]
    fn the_median_needs_three_mornings_and_rounds_half_up() {
        assert_eq!(median_resting_hr(&[]), None, "no mornings, no median");
        assert_eq!(
            median_resting_hr(&[50, 52]),
            None,
            "two mornings cannot stand for a week"
        );
        assert_eq!(median_resting_hr(&[52, 49, 70]), Some(52), "odd count");
        assert_eq!(
            median_resting_hr(&[48, 51, 52, 60]),
            Some(52),
            "51.5 rounds half up"
        );
        assert_eq!(
            median_resting_hr(&[50, 52, 52, 90, 51, 49, 50]),
            Some(51),
            "a spike does not move the median"
        );
    }

    #[test]
    fn only_an_empty_estimated_or_promoted_anchor_is_promotable() {
        assert_eq!(
            promotable_anchors(None),
            Anchors {
                resting_hr: true,
                vo2_max: true,
                weight: true
            },
            "no profile: every anchor is empty"
        );
        let mut held = profile();
        held.resting_hr = Some(48);
        held.vo2_max = Some(55.0);
        held.weight = Some(70.0);
        assert_eq!(
            promotable_anchors(Some(&held)),
            Anchors::default(),
            "values stored before origins were recorded are the athlete's own"
        );
        held.anchor_provenance.resting_hr = Some(AnchorStamp {
            origin: AnchorOrigin::Estimated,
            as_of: None,
        });
        held.anchor_provenance.vo2_max = Some(AnchorStamp {
            origin: AnchorOrigin::Imported,
            as_of: None,
        });
        held.anchor_provenance.weight = Some(AnchorStamp::promoted(day(20)));
        assert_eq!(
            promotable_anchors(Some(&held)),
            Anchors {
                resting_hr: true,
                vo2_max: false,
                weight: true
            },
            "an estimate and a promotion are refreshed; an import stands"
        );
    }

    const fn measured<T>(value: T, as_of: NaiveDate) -> Measured<T> {
        Measured {
            value,
            as_of,
            policy: TransportPolicy::AnyTransport,
        }
    }

    #[test]
    fn a_promotion_never_lands_on_the_athletes_number() {
        let mut stated = profile();
        stated.resting_hr = Some(46);
        stated.anchor_provenance.resting_hr = Some(AnchorStamp::athlete(day(1)));
        let promotion = Promotion {
            resting_hr: Some(measured(52, day(30))),
            weight: Some(measured(71.5, day(29))),
            vo2_max: None,
        };
        assert_eq!(
            apply_promotion(&mut stated, &promotion),
            Anchors {
                weight: true,
                ..Anchors::default()
            },
            "only the empty weight moves"
        );
        assert_eq!(stated.resting_hr, Some(46), "the athlete's RHR stands");
        assert_eq!(
            stated.anchor_provenance.resting_hr,
            Some(AnchorStamp::athlete(day(1))),
            "bug: the athlete's stamp was replaced"
        );
        assert_eq!(
            stated.weight,
            Some(71.5),
            "bug: the empty weight was not filled"
        );
        assert_eq!(
            stated.anchor_provenance.weight,
            Some(AnchorStamp::promoted(day(29))),
            "bug: the filled weight is not stamped as promoted"
        );
        assert_eq!(
            apply_promotion(&mut stated, &promotion),
            Anchors::default(),
            "the same promotion twice changes nothing"
        );
        let same_day = Promotion {
            weight: Some(measured(72.0, day(29))),
            ..promotion
        };
        assert_eq!(
            apply_promotion(&mut stated, &same_day),
            Anchors::default(),
            "a different value as of the promoted day is not applied"
        );
        assert_eq!(stated.weight, Some(71.5), "bug: a same-day value moved");
        let newer = Promotion {
            weight: Some(measured(71.0, day(31))),
            ..promotion
        };
        assert!(
            apply_promotion(&mut stated, &newer).weight,
            "a newer weigh-in refreshes the promoted weight"
        );
        assert_eq!(
            stated.weight,
            Some(71.0),
            "bug: the promoted weight was not refreshed"
        );
    }

    #[test]
    fn only_the_anchors_that_moved_stamp_the_profile() {
        let first_party = |value: u16| {
            Some(Measured {
                value,
                as_of: day(30),
                policy: TransportPolicy::FirstPartyOnly,
            })
        };
        let promotion = Promotion {
            resting_hr: first_party(52),
            weight: Some(measured(70.0, day(30))),
            vo2_max: None,
        };
        assert_eq!(
            promotion.policy_of(Anchors {
                weight: true,
                ..Anchors::default()
            }),
            TransportPolicy::AnyTransport,
            "bug: rows read for a resting heart rate that did not move stamped the profile"
        );
        assert_eq!(
            promotion.policy_of(Anchors {
                resting_hr: true,
                weight: true,
                vo2_max: false,
            }),
            TransportPolicy::FirstPartyOnly,
            "a first-party-only anchor that moved stamps the profile"
        );
    }
}
