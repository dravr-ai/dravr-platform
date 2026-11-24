// ABOUTME: The athlete's one VDOT: a stated VDOT while fresh, else their best recent run, else their saved threshold pace
// ABOUTME: Builds cageux's VdotInputs from the stored profile, so the verifier, the zones and predict_performance read one VDOT

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The athlete's VDOT (carnet#1693 item 5)
//!
//! A VDOT comes from a performance only, never from a `VO2max`: cageux's
//! [`PerformancePredictor::resolve_vdot`] reads, in order, a VDOT the athlete
//! or coach stated while it is at most 12 weeks old, the highest-VDOT run of
//! the last 6 weeks (an estimate), the VDOT the saved threshold pace implies
//! (an estimate), or none. [`resolve_athlete_vdot`] feeds it the stored
//! profile, so the claim verifier's snapshot, `calculate_personalized_zones`,
//! `predict_performance` and `generate_recommendations` all read the same one.

use dravr_cageux::algorithms::DANIELS_TABLE_VDOT;
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::performance_prediction::{
    PerformancePredictor, StatedVdot, VdotInputs, VdotReading, VdotSource, RECENT_RUN_WINDOW_DAYS,
};
use dravr_cageux::physiological_constants::physiological_defaults::DEFAULT_LACTATE_THRESHOLD;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId, UserPhysiologicalProfile};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use std::slice;
use uuid::Uuid;

/// Most cached activities read for the recent-run window: six weeks of two
/// sessions a day, with room to spare.
const RECENT_RUN_READ_LIMIT: i64 = 200;

/// The VDOT the profile states, dated for cageux: its day read as midnight
/// UTC. `None` when the profile states none.
fn stated_vdot(profile: Option<&UserPhysiologicalProfile>) -> Option<StatedVdot> {
    let vdot = profile?.vdot.as_ref()?;
    let as_of = vdot.as_of.at_midnight().and_utc().ok()?;
    Some(StatedVdot {
        vdot: vdot.value,
        as_of,
    })
}

/// The warning a VDOT outside Daniels' tables (30-85) carries.
///
/// Such a VDOT rests on his curves beyond the races his tables print, and is
/// less certain. `None` inside them. Every tool that reports or uses a VDOT
/// says it the same way.
#[must_use]
pub fn vdot_beyond_tables_warning(vdot: f64) -> Option<String> {
    (!DANIELS_TABLE_VDOT.contains(&vdot)).then(|| {
        format!(
            "VDOT {vdot:.1} is outside the {:.0}-{:.0} Daniels' tables cover: it rests on his \
             curves beyond them and is less certain",
            DANIELS_TABLE_VDOT.start(),
            DANIELS_TABLE_VDOT.end()
        )
    })
}

/// The athlete's threshold share of VDOT: the profile's lactate threshold,
/// else Daniels' 0.88.
#[must_use]
pub fn lactate_threshold(profile: Option<&UserPhysiologicalProfile>) -> f64 {
    profile
        .and_then(|p| p.lactate_threshold_percentage)
        .unwrap_or(DEFAULT_LACTATE_THRESHOLD)
}

/// The athlete's VDOT as of `now`, from their stored profile and
/// `activities` (only runs are read), with where it came from; `None` when
/// no performance gives one. A `VO2max` is never read.
#[must_use]
pub fn resolve_athlete_vdot(
    profile: Option<&UserPhysiologicalProfile>,
    activities: &[Activity],
    now: Timestamp,
    config: &AlgorithmConfig,
) -> Option<VdotReading> {
    PerformancePredictor::resolve_vdot(
        &VdotInputs {
            stated: stated_vdot(profile),
            activities,
            threshold_pace_sec_per_km: profile.and_then(|p| p.threshold_pace_sec_per_km),
            lactate_threshold: lactate_threshold(profile),
            now,
        },
        config,
    )
}

/// The athlete's VDOT as of `now` for a reader with no activity read of its
/// own: [`resolve_athlete_vdot`] over the runs sync cached.
///
/// A fresh stated VDOT is returned without reading the cache. Otherwise the
/// cached runs pass the AI terms filter (carnet#723) one at a time, best
/// first, until one comes back that still gives a VDOT, so no run ranked
/// below it, and no activity that could never give one, is recorded as
/// served in the trust label and the attribution the result carries. A run
/// the filter withholds, or reduces past what a VDOT is read from, gives way
/// to the next best. A database read of what sync already stored: no
/// provider is called.
///
/// # Errors
///
/// Returns the repository's error when the cache cannot be read.
pub async fn resolve_athlete_vdot_from_cache(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    tenant_id: TenantId,
    user_id: Uuid,
    profile: Option<&UserPhysiologicalProfile>,
    now: Timestamp,
    config: &AlgorithmConfig,
) -> AppResult<Option<VdotReading>> {
    if let Some(stated) = resolve_athlete_vdot(profile, &[], now, config)
        .filter(|reading| matches!(reading.source, VdotSource::Stated { .. }))
    {
        return Ok(Some(stated));
    }
    let cached = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            None,
            now - Elapsed::days(RECENT_RUN_WINDOW_DAYS),
            now,
            RECENT_RUN_READ_LIMIT,
        )
        .await?;
    let run = best_admitted_run(terms, cached, now, config);
    Ok(resolve_athlete_vdot(profile, run.as_slice(), now, config))
}

/// The highest-VDOT run among `candidates` that `terms` let an AI reader see.
///
/// Each candidate is read through cageux's own [`PerformancePredictor::best_recent_run`]
/// on its own, so only a run it would count is ranked. Each is then filtered
/// alone, best first, until the filter hands one back that cageux still reads
/// as a run, so the filter records none ranked below it. The filter only
/// removes fields, so a reduced run cageux still reads keeps the VDOT it was
/// ranked by.
fn best_admitted_run(
    terms: &dyn ProviderTerms,
    candidates: Vec<Activity>,
    now: Timestamp,
    config: &AlgorithmConfig,
) -> Option<Activity> {
    let mut ranked: Vec<(f64, Activity)> = candidates
        .into_iter()
        .filter_map(|activity| {
            PerformancePredictor::best_recent_run(slice::from_ref(&activity), now, config)
                .map(|run| (run.vdot, activity))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked.into_iter().find_map(|(_, activity)| {
        ai_scope::filter_activities(terms, vec![activity])
            .pop()
            .filter(|kept| {
                PerformancePredictor::best_recent_run(slice::from_ref(kept), now, config).is_some()
            })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use dravr_tronc::dates::CivilDate;
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::models::{ActivityBuilder, ProfileVdot, SportType};
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_providers::provider_terms;

    use super::*;

    /// Nolio's own terms, for the one provider they name.
    ///
    /// No provider serves Nolio yet (registre#657), and no other terms reduce
    /// a run past its distance, so this lookup is the one way such a run
    /// reaches the candidates.
    struct NolioTerms;

    impl ProviderTerms for NolioTerms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == "nolio").then_some(&provider_terms::NOLIO)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == "nolio").then_some(provider_terms::NOLIO_TRANSPORT)
        }
    }

    fn profile_stating(vdot: f64, as_of: CivilDate) -> UserPhysiologicalProfile {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.vdot = Some(ProfileVdot {
            value: vdot,
            as_of,
            method: Some("stated".to_owned()),
        });
        profile
    }

    fn day(y: i32, m: u32, d: u32) -> CivilDate {
        CivilDate::from_ymd(y, m, d).unwrap_or_default()
    }

    fn noon(y: i32, m: u32, d: u32) -> Timestamp {
        day(y, m, d)
            .and_hms(12, 0, 0)
            .ok()
            .and_then(|t| t.and_utc().ok())
            .unwrap_or_else(Timestamp::now)
    }

    #[test]
    fn a_fresh_stated_vdot_wins_and_names_its_day() {
        let profile = profile_stating(52.0, day(2026, 11, 1));
        let reading = resolve_athlete_vdot(
            Some(&profile),
            &[],
            noon(2026, 11, 22),
            &AlgorithmConfig::default(),
        );
        let reading = reading.unwrap_or_else(|| panic!("a fresh stated VDOT resolves"));
        assert!(
            (reading.vdot - 52.0).abs() < f64::EPSILON,
            "bug: the stated VDOT was not read as given: {}",
            reading.vdot
        );
        assert!(
            matches!(reading.source, VdotSource::Stated { .. }),
            "bug: a fresh stated VDOT must name itself as stated"
        );
    }

    #[test]
    fn only_a_vdot_beyond_daniels_tables_carries_the_warning() {
        assert!(
            vdot_beyond_tables_warning(30.0).is_none()
                && vdot_beyond_tables_warning(85.0).is_none(),
            "bug: the table's own edges are inside it"
        );
        let below = vdot_beyond_tables_warning(25.6).unwrap_or_default();
        assert!(
            below.contains("25.6") && below.contains("less certain"),
            "bug: a VDOT below the tables must say it is less certain: {below}"
        );
        assert!(
            vdot_beyond_tables_warning(85.1).is_some(),
            "bug: a VDOT above the tables must carry the warning"
        );
    }

    #[test]
    fn a_vo2max_alone_gives_no_vdot() {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.vo2_max = Some(60.0);
        assert!(
            resolve_athlete_vdot(
                Some(&profile),
                &[],
                noon(2026, 11, 22),
                &AlgorithmConfig::default(),
            )
            .is_none(),
            "bug: a VO2max was read as a VDOT"
        );
    }

    #[test]
    fn a_saved_threshold_pace_is_read_back_as_an_estimate() {
        let mut profile = profile_stating(52.0, day(2026, 6, 1));
        // 4:15/km at Daniels' 0.88 is VDOT 50; the June VDOT is past 12 weeks.
        profile.threshold_pace_sec_per_km = Some(255.0);
        let reading = resolve_athlete_vdot(
            Some(&profile),
            &[],
            noon(2026, 11, 22),
            &AlgorithmConfig::default(),
        )
        .unwrap_or_else(|| panic!("the threshold pace resolves"));
        assert!(
            (reading.vdot - 50.0).abs() < 0.5,
            "bug: 4:15/km at 0.88 must read back as VDOT 50, got {}",
            reading.vdot
        );
        assert!(
            matches!(reading.source, VdotSource::ThresholdPace { .. }),
            "bug: a stale stated VDOT must yield to the threshold pace"
        );
    }

    #[tokio::test]
    async fn a_run_the_terms_reduce_past_its_distance_gives_way_to_the_next_best() {
        let now = noon(2026, 11, 22);
        let ten_km = |id: &str, provider: &str, seconds: u64| {
            ActivityBuilder::new(
                id,
                "10 km",
                SportType::Run,
                now - Elapsed::days(2),
                seconds,
                provider,
            )
            .distance_meters(10_000.0)
        };
        // Nolio relaying a Strava 10 km in 40 minutes: an AI reader may see
        // that it happened, never how far it went.
        let relayed = ten_km("relayed", "nolio", 2_400).source("strava").build();
        let uploaded = ten_km("uploaded", "upload", 3_000).build();
        let (kept, withheld) = Box::pin(ai_scope::serve_over(
            Transport::WebApp,
            ai_scope::with_ai_consent(
                BTreeSet::new(),
                ai_scope::ai_read(async {
                    best_admitted_run(
                        &NolioTerms,
                        vec![relayed, uploaded],
                        now,
                        &AlgorithmConfig::default(),
                    )
                }),
            ),
        ))
        .await;
        assert_eq!(
            withheld.reduced, 1,
            "fixture: the terms reduce the relayed run to its existence"
        );
        assert_eq!(
            kept.as_ref().map(Activity::id),
            Some("uploaded"),
            "bug: a run reduced past its distance gives no VDOT, so the next best must give it"
        );
    }
}
