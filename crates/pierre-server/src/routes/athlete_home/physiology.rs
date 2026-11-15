// ABOUTME: GET /api/me/physiology — the athlete's own anchors per sport family, each with its range, source and date
// ABOUTME: Read through the one anchors reader and worded as the envelope notice words an anchor; confidence as stored

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's physiology (carnet#1516).
//!
//! `GET /api/me/physiology?locale=xx` answers every anchor the athlete
//! holds, per sport family — ride, run, swim, in that order, a family
//! holding none left out — as
//! [`stored_athlete_anchors`] reads them: the per-sport anchors stored
//! beside the profile, and the profile row's thresholds, each on the one
//! family it belongs to at the low confidence the reader maps it at.
//! Nothing here derives a confidence, a range or a value.
//!
//! Each anchor carries what is stored — value, range, origin, day, method
//! and confidence — and the words the screen prints, through the same
//! formatter the envelope notice names an anchor with: its value (a range
//! or an estimate at low confidence; a speed as pace per kilometre or mile
//! as the athlete's units say, per 100 m in the pool), the range it states
//! when the value does not already read as it, and who set it.
//!
//! The caller's own anchors only, in the tenant of their session: nothing
//! in the request names an athlete. An athlete with no profile, or none
//! this transport may read, holds no anchor, and `sports` is empty.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use chrono::NaiveDate;
use pierre_contremaitre::anchor_strings::{
    anchor_range_text, anchor_source_text, anchor_value_text, AnchorReader,
};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::AppResult;
use pierre_core::models::periodization::{
    AnchorConfidence, AnchorValue, AthleteAnchors, SportFamily,
};
use pierre_core::models::{AnchorUnit, SportAnchorName, UnitSystem};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::athlete_anchors::stored_athlete_anchors;
use pierre_services::locale::user_locale;
use pierre_services::personas::resolve_persona_locale;
use pierre_services::units::resolve_user_units;
use serde::{Deserialize, Serialize};

use super::active_tenant;
use crate::mcp::resources::ServerContext;

/// Query parameters for `GET /api/me/physiology`.
#[derive(Debug, Deserialize)]
pub(super) struct PhysiologyQuery {
    /// The locale the anchors' words render in; an unsupported or absent
    /// value falls back to the athlete's stored locale, then to the default.
    #[serde(default)]
    locale: Option<String>,
}

/// Body of `GET /api/me/physiology`.
#[derive(Debug, Clone, Serialize)]
pub(super) struct PhysiologyResponse {
    /// Each sport family the athlete holds an anchor in: ride, run, swim.
    sports: Vec<SportPhysiology>,
}

/// One sport family's anchors.
#[derive(Debug, Clone, Serialize)]
struct SportPhysiology {
    /// The family: `ride`, `run` or `swim`.
    sport: SportFamily,
    /// Its anchors, in the order [`SportAnchorName::ALL`] declares them.
    anchors: Vec<PhysiologyAnchor>,
}

/// One anchor as it is stored, and as the screen words it. Every key is
/// always present; an absent value is `null`.
#[derive(Debug, Clone, Serialize)]
struct PhysiologyAnchor {
    /// Which anchor it is.
    anchor: SportAnchorName,
    /// The unit `value`, `low` and `high` are in.
    unit: AnchorUnit,
    /// The stored value.
    value: f32,
    /// The low end of the range it stands for, when one is stated.
    low: Option<f32>,
    /// The high end of the range it stands for, when one is stated.
    high: Option<f32>,
    /// Who set it, as stored: `athlete`, a provider, a test.
    origin: Option<String>,
    /// The day it was measured, stated or imported, when known.
    as_of: Option<NaiveDate>,
    /// How it was measured, as stored; for MAP, its definition.
    method: Option<String>,
    /// How far to trust it, as stored.
    confidence: AnchorConfidence,
    /// The value as the envelope notice writes it: a range or an estimate at
    /// low confidence, a speed as pace in the athlete's units.
    value_text: String,
    /// The range it states, when `value_text` does not already read as it;
    /// `null` at low confidence, where the value is the range.
    range_text: Option<String>,
    /// Who set it, as the athlete reads it: their own value, or "from" its
    /// origin.
    source_text: String,
}

/// `GET /api/me/physiology`.
pub(super) async fn get_physiology(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<PhysiologyQuery>,
) -> AppResult<Json<PhysiologyResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let repos = &resources.common.repos;
    let user = repos.users.get_global(user_id).await?;
    let locale = resolve_persona_locale(query.locale.as_deref(), Some(&user_locale(user.as_ref())));
    let units = resolve_user_units(repos.unit_preferences.as_ref(), user_id).await;
    let sports = stored_athlete_anchors(repos, tenant_id, user_id)
        .await?
        .map_or_else(Vec::new, |held| {
            sports_of(
                &held,
                &resources.mcp.messaging_strings_registry,
                &locale,
                units,
            )
        });
    Ok(Json(PhysiologyResponse { sports }))
}

/// The families of `anchors` that hold an anchor, each with its anchors
/// worded in `locale` and `units`.
fn sports_of(
    anchors: &AthleteAnchors,
    strings: &MessagingStringsRegistry,
    locale: &str,
    units: UnitSystem,
) -> Vec<SportPhysiology> {
    SportFamily::ALL
        .iter()
        .copied()
        .filter_map(|sport| {
            let held = anchors.for_family(sport);
            let rows: Vec<PhysiologyAnchor> = SportAnchorName::ALL
                .into_iter()
                .filter_map(|name| {
                    name.get(held).map(|point| PhysiologyAnchor {
                        anchor: name,
                        unit: name.unit(),
                        value_text: anchor_value_text(
                            strings,
                            locale,
                            name.unit(),
                            sport,
                            units,
                            point,
                        ),
                        range_text: range_beside_value(name.unit(), sport, units, point),
                        source_text: anchor_source_text(
                            strings,
                            locale,
                            AnchorReader::Athlete,
                            point.origin.as_deref(),
                        ),
                        value: point.value,
                        low: point.low,
                        high: point.high,
                        origin: point.origin.clone(),
                        as_of: point.as_of,
                        method: point.method.clone(),
                        confidence: point.confidence,
                    })
                })
                .collect();
            (!rows.is_empty()).then_some(SportPhysiology {
                sport,
                anchors: rows,
            })
        })
        .collect()
}

/// The range `point` states, for a value that reads as a point: at low
/// confidence the value already reads as its range, or as an estimate.
fn range_beside_value(
    unit: AnchorUnit,
    sport: SportFamily,
    units: UnitSystem,
    point: &AnchorValue,
) -> Option<String> {
    if point.confidence == AnchorConfidence::Low {
        return None;
    }
    anchor_range_text(unit, sport, units, point)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(value: f32, range: Option<(f32, f32)>, confidence: AnchorConfidence) -> AnchorValue {
        AnchorValue {
            value,
            low: range.map(|(low, _)| low),
            high: range.map(|(_, high)| high),
            origin: Some("intervals.icu".to_owned()),
            as_of: NaiveDate::from_ymd_opt(2026, 10, 1),
            method: None,
            confidence,
        }
    }

    #[test]
    fn only_the_families_holding_an_anchor_are_listed_in_order() {
        let strings = MessagingStringsRegistry::new();
        let mut anchors = AthleteAnchors::default();
        anchors.swim.critical_speed = Some(point(1.25, None, AnchorConfidence::Medium));
        anchors.ride.map = Some(point(380.0, Some((365.0, 395.0)), AnchorConfidence::High));
        anchors.ride.ftp = Some(point(285.0, None, AnchorConfidence::Low));

        let sports = sports_of(&anchors, &strings, "en", UnitSystem::Metric);

        let families: Vec<SportFamily> = sports.iter().map(|s| s.sport).collect();
        assert_eq!(
            families,
            [SportFamily::Ride, SportFamily::Swim],
            "bug: a family with no anchor is listed, or the families are out of order"
        );
        let ride: Vec<SportAnchorName> = sports[0].anchors.iter().map(|a| a.anchor).collect();
        assert_eq!(
            ride,
            [SportAnchorName::Ftp, SportAnchorName::Map],
            "bug: a family's anchors follow the declared order"
        );
        assert_eq!(
            sports[1].anchors[0].value_text, "1:20/100 m",
            "bug: a pool speed reads as pace per 100 m"
        );
    }

    #[test]
    fn a_stated_range_shows_beside_a_point_and_never_twice() {
        let strings = MessagingStringsRegistry::new();
        let mut anchors = AthleteAnchors::default();
        anchors.ride.map = Some(point(380.0, Some((365.0, 395.0)), AnchorConfidence::High));
        anchors.ride.critical_power =
            Some(point(280.0, Some((270.0, 290.0)), AnchorConfidence::Low));

        let sports = sports_of(&anchors, &strings, "en", UnitSystem::Metric);
        let ride = &sports[0].anchors;

        let cp = &ride[0];
        assert_eq!(
            cp.anchor,
            SportAnchorName::CriticalPower,
            "bug: critical power is declared before MAP, so it opens the ride family"
        );
        assert_eq!(
            cp.value_text, "about 270–290 W (estimate)",
            "bug: a low-confidence anchor reads as its range"
        );
        assert_eq!(
            cp.range_text, None,
            "bug: the range is shown twice when the value already reads as it"
        );
        assert_eq!(
            cp.source_text, "from intervals.icu",
            "bug: an anchor a provider produced names it"
        );

        let map = &ride[1];
        assert_eq!(
            map.value_text, "380 W",
            "bug: a high-confidence anchor reads as its value"
        );
        assert_eq!(
            map.range_text.as_deref(),
            Some("365–395 W"),
            "bug: a high-confidence anchor's stated range is dropped"
        );
        assert_eq!(
            map.confidence,
            AnchorConfidence::High,
            "bug: the confidence is the stored one, never derived"
        );
    }

    #[test]
    fn an_imperial_athlete_reads_run_pace_per_mile() {
        let strings = MessagingStringsRegistry::new();
        let mut anchors = AthleteAnchors::default();
        anchors.run.threshold_speed = Some(AnchorValue {
            origin: None,
            ..point(4.0, None, AnchorConfidence::Medium)
        });

        let sports = sports_of(&anchors, &strings, "en", UnitSystem::Imperial);
        let threshold = &sports[0].anchors[0];

        assert_eq!(
            threshold.value_text, "6:42/mi",
            "bug: run pace follows the athlete's imperial setting"
        );
        assert_eq!(
            threshold.source_text, "your own value",
            "bug: an anchor naming no origin is the athlete's own"
        );
    }
}
