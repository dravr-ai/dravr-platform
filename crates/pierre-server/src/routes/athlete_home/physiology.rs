// ABOUTME: GET /api/me/physiology — the athlete's own anchors per sport family, and the PUT's edit module that sets one at a time
// ABOUTME: Read through the one anchors reader, worded as the envelope notice words them; the response every physiology route answers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's physiology (carnet#1516, carnet#1561).
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
//! or an estimate at low confidence, an estimate at any confidence unless
//! the anchor was measured; a speed as pace per kilometre or mile as the
//! athlete's units say, per 100 m in the pool), the range it states when the
//! value does not already read as it, and who set it.
//!
//! The caller's own anchors only, in the tenant of their session: nothing
//! in the request names an athlete. An athlete with no profile, or none
//! this transport may read, holds no anchor, and `sports` is empty. Beside
//! them, `offered` lists every anchor each family can hold, with its unit:
//! what an edit may set.
//!
//! `PUT /api/me/physiology/{sport}/{anchor}?locale=xx` sets one anchor and
//! answers with the anchors as the `GET` does; it, and what the coach's edit
//! shares with it, is `edit`, built with the `tools-config` feature that
//! builds the physiology writer it saves through. The `GET` reads the stored
//! anchors whatever that feature or the write tool's switch says.

/// The edit of one anchor, the athlete's own and the coach's: built only
/// where its writer is.
#[cfg(feature = "tools-config")]
pub(crate) mod edit;

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use dravr_tronc::dates::CivilDate;
use pierre_contremaitre::anchor_strings::{
    anchor_range_text, anchor_source_text, anchor_value_text, AnchorReader,
};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::AppResult;
use pierre_core::models::periodization::{AnchorConfidence, AnchorValue, SportFamily};
use pierre_core::models::{AnchorUnit, HeldAnchors, SportAnchorName, TenantId, UnitSystem};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_runtime_context::require_tenant;
use pierre_services::athlete_anchors::stored_athlete_anchors;
use pierre_services::locale::user_locale;
use pierre_services::personas::resolve_persona_locale;
use pierre_services::units::resolve_user_units;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Query parameters for `GET` and `PUT /api/me/physiology`.
#[derive(Debug, Deserialize)]
pub(super) struct PhysiologyQuery {
    /// The locale the anchors' words render in; an unsupported or absent
    /// value falls back to the athlete's stored locale, then to the default.
    #[serde(default)]
    locale: Option<String>,
}

/// Body of `GET /api/me/physiology`, of `PUT /api/me/physiology/{sport}/{anchor}`
/// and of the coach's view of one athlete's anchors.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PhysiologyResponse {
    /// Each sport family the athlete holds an anchor in: ride, run, swim.
    sports: Vec<SportPhysiology>,
    /// Every anchor each family can hold, with its unit: what an edit may
    /// set. Ride, run, swim; each family's anchors in the order
    /// [`SportAnchorName::ALL`] declares them.
    offered: Vec<OfferedFamily>,
}

/// The anchors one sport family can hold.
#[derive(Debug, Clone, Serialize)]
struct OfferedFamily {
    /// The family: `ride`, `run` or `swim`.
    sport: SportFamily,
    /// Its anchors, each with its unit.
    anchors: Vec<OfferedAnchor>,
}

/// One anchor a family can hold.
#[derive(Debug, Clone, Serialize)]
struct OfferedAnchor {
    /// Which anchor.
    anchor: SportAnchorName,
    /// The unit it is stored in; a speed is entered as a pace.
    unit: AnchorUnit,
}

/// `athlete`'s anchors in `tenant`, worded for `reader_user` reading as
/// `reader`: in that reader's locale (or `locale`) and units.
///
/// # Errors
///
/// Returns the repository error of the reader's account, the athlete's
/// profile or their anchors.
pub(crate) async fn physiology_response(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    athlete: Uuid,
    reader_user: Uuid,
    reader: AnchorReader,
    locale: Option<&str>,
) -> AppResult<PhysiologyResponse> {
    let repos = &resources.common.repos;
    let user = repos.users.get_global(reader_user).await?;
    let locale = resolve_persona_locale(locale, Some(&user_locale(user.as_ref())));
    let units = resolve_user_units(repos.unit_preferences.as_ref(), reader_user).await;
    let sports = stored_athlete_anchors(repos, tenant, athlete)
        .await?
        .map_or_else(Vec::new, |held| {
            sports_of(
                &held,
                &resources.mcp.messaging_strings_registry,
                &locale,
                units,
                reader,
            )
        });
    Ok(PhysiologyResponse {
        sports,
        offered: offered(),
    })
}

/// Every anchor each family can hold, with its unit.
fn offered() -> Vec<OfferedFamily> {
    SportFamily::ALL
        .iter()
        .copied()
        .map(|sport| OfferedFamily {
            sport,
            anchors: SportAnchorName::ALL
                .into_iter()
                .filter(|name| name.offered_in(sport))
                .map(|anchor| OfferedAnchor {
                    anchor,
                    unit: anchor.unit(),
                })
                .collect(),
        })
        .collect()
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
    as_of: Option<CivilDate>,
    /// How it was measured, as stored; for MAP, its definition.
    method: Option<String>,
    /// How far to trust it, as stored.
    confidence: AnchorConfidence,
    /// The value as the envelope notice writes it: a range or an estimate at
    /// low confidence, an estimate at any confidence unless measured, a speed
    /// as pace in the reader's units.
    value_text: String,
    /// The range it states, when `value_text` does not already read as it;
    /// `null` at low confidence, where the value is the range.
    range_text: Option<String>,
    /// Who set it, as its reader reads it: their own value (the athlete's own
    /// in a coach's view), words for an origin stored as a code (`imported`,
    /// `estimated`, `promoted`), set by a coach, or "from" the provider, app
    /// or test it names.
    source_text: String,
}

/// `GET /api/me/physiology`.
pub(super) async fn get_physiology(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<PhysiologyQuery>,
) -> AppResult<Json<PhysiologyResponse>> {
    let tenant_id = require_tenant(&resources, &auth).await?;
    let body = physiology_response(
        &resources,
        tenant_id,
        auth.user_id,
        auth.user_id,
        AnchorReader::Athlete,
        query.locale.as_deref(),
    )
    .await?;
    Ok(Json(body))
}

/// The families of `anchors` that hold an anchor, each with its anchors
/// worded in `locale` and `units` for `reader`.
fn sports_of(
    anchors: &HeldAnchors,
    strings: &MessagingStringsRegistry,
    locale: &str,
    units: UnitSystem,
    reader: AnchorReader,
) -> Vec<SportPhysiology> {
    SportFamily::ALL
        .iter()
        .copied()
        .filter_map(|sport| {
            let held = anchors.values().for_family(sport);
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
                            anchors.kind(sport, name),
                        ),
                        range_text: range_beside_value(name.unit(), sport, units, point),
                        source_text: anchor_source_text(
                            strings,
                            locale,
                            reader,
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
    use pierre_core::models::{
        sport_anchors, MeasurementKind, SportAnchor, SportType, UserPhysiologicalProfile,
    };
    use uuid::Uuid;

    fn point(value: f32, range: Option<(f32, f32)>, confidence: AnchorConfidence) -> AnchorValue {
        AnchorValue {
            value,
            low: range.map(|(low, _)| low),
            high: range.map(|(_, high)| high),
            origin: Some("intervals.icu".to_owned()),
            as_of: CivilDate::from_ymd(2026, 10, 1).ok(),
            method: None,
            confidence,
        }
    }

    fn stored(
        sport: SportFamily,
        anchor: SportAnchorName,
        kind: MeasurementKind,
        value: AnchorValue,
    ) -> SportAnchor {
        SportAnchor {
            sport,
            anchor,
            kind,
            value,
        }
    }

    /// `stored` as the one reader returns it beside a profile row that holds
    /// nothing of its own.
    fn held(stored: &[SportAnchor]) -> HeldAnchors {
        let profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        sport_anchors::held_anchors(&profile, stored)
    }

    #[test]
    fn only_the_families_holding_an_anchor_are_listed_in_order() {
        let strings = MessagingStringsRegistry::new();
        let anchors = held(&[
            stored(
                SportFamily::Swim,
                SportAnchorName::CriticalSpeed,
                MeasurementKind::Measured,
                point(1.25, None, AnchorConfidence::Medium),
            ),
            stored(
                SportFamily::Ride,
                SportAnchorName::Map,
                MeasurementKind::Measured,
                point(380.0, Some((365.0, 395.0)), AnchorConfidence::High),
            ),
            stored(
                SportFamily::Ride,
                SportAnchorName::Ftp,
                MeasurementKind::Estimated,
                point(285.0, None, AnchorConfidence::Low),
            ),
        ]);

        let sports = sports_of(
            &anchors,
            &strings,
            "en",
            UnitSystem::Metric,
            AnchorReader::Athlete,
        );

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
        let anchors = held(&[
            stored(
                SportFamily::Ride,
                SportAnchorName::Map,
                MeasurementKind::Measured,
                point(380.0, Some((365.0, 395.0)), AnchorConfidence::High),
            ),
            stored(
                SportFamily::Ride,
                SportAnchorName::CriticalPower,
                MeasurementKind::Measured,
                point(280.0, Some((270.0, 290.0)), AnchorConfidence::Low),
            ),
        ]);

        let sports = sports_of(
            &anchors,
            &strings,
            "en",
            UnitSystem::Metric,
            AnchorReader::Athlete,
        );
        let ride = &sports[0].anchors;

        let cp = &ride[0];
        assert_eq!(
            cp.anchor,
            SportAnchorName::CriticalPower,
            "bug: critical power is declared before MAP, so it opens the ride family"
        );
        assert_eq!(
            cp.value_text, "about 270–290 W (estimate)",
            "bug: a low-confidence anchor reads as its range, measured or not"
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
            "bug: a measured high-confidence anchor reads as its value"
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
        let anchors = held(&[stored(
            SportFamily::Run,
            SportAnchorName::ThresholdSpeed,
            MeasurementKind::Measured,
            AnchorValue {
                origin: None,
                ..point(4.0, None, AnchorConfidence::Medium)
            },
        )]);

        let sports = sports_of(
            &anchors,
            &strings,
            "en",
            UnitSystem::Imperial,
            AnchorReader::Athlete,
        );
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

    #[test]
    fn an_estimate_at_high_confidence_reads_as_one_with_its_range_beside() {
        let strings = MessagingStringsRegistry::new();
        let anchors = held(&[
            stored(
                SportFamily::Ride,
                SportAnchorName::CriticalPower,
                MeasurementKind::Estimated,
                point(280.0, Some((270.0, 290.0)), AnchorConfidence::High),
            ),
            stored(
                SportFamily::Ride,
                SportAnchorName::Map,
                MeasurementKind::Measured,
                point(380.0, None, AnchorConfidence::High),
            ),
        ]);

        let sports = sports_of(
            &anchors,
            &strings,
            "en",
            UnitSystem::Metric,
            AnchorReader::Athlete,
        );
        let ride = &sports[0].anchors;

        let cp = &ride[0];
        assert_eq!(
            cp.value_text, "about 280 W (estimate)",
            "bug: an estimate stored at high confidence reads as a measured number"
        );
        assert_eq!(
            cp.range_text.as_deref(),
            Some("270–290 W"),
            "bug: a high-confidence estimate still shows the range it states beside it"
        );
        assert_eq!(
            cp.confidence,
            AnchorConfidence::High,
            "bug: the confidence is the stored one, never lowered for an estimate"
        );
        assert_eq!(
            ride[1].value_text, "380 W",
            "bug: a measured high-confidence anchor still reads as its value"
        );
    }

    #[test]
    fn every_family_offers_the_anchors_it_can_hold() {
        let families = offered();
        let names = |sport: SportFamily| -> Vec<SportAnchorName> {
            families
                .iter()
                .find(|family| family.sport == sport)
                .map_or_default(|family| family.anchors.iter().map(|a| a.anchor).collect())
        };
        assert!(
            !names(SportFamily::Ride).contains(&SportAnchorName::CriticalSpeed),
            "bug: the bike holds no speed anchor"
        );
        assert!(
            !names(SportFamily::Swim).contains(&SportAnchorName::Ftp),
            "bug: the pool holds no power anchor"
        );
        assert_eq!(
            names(SportFamily::Run).len(),
            SportAnchorName::ALL.len(),
            "bug: the run holds power, speed and heart rate"
        );
    }
}
