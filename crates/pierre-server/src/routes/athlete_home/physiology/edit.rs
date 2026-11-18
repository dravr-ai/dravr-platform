// ABOUTME: PUT /api/me/physiology/{sport}/{anchor} — the athlete sets one of their own anchors through the one physiology writer
// ABOUTME: The edit body, its pace conversion and the tool switch the athlete's and the coach's edits share; built with tools-config

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The edit of one physiology anchor (carnet#1561).
//!
//! `PUT /api/me/physiology/{sport}/{anchor}?locale=xx` sets one anchor
//! ([`AnchorEdit`]) and answers with the anchors as the `GET` does. It
//! saves through [`save_physiology`], the writer `set_physiology` saves
//! through, with the anchor stated as that tool's `sport_anchors` entry: the
//! same bounds, the same mirroring onto the profile field a twin anchor
//! shares, the same refusal of a value that contradicts a stored one. The
//! athlete is its source (origin `athlete`), at the confidence they state,
//! low when they state none. A group's human coach sets one of their
//! athletes' anchors the same way from Group info
//! (`routes::chat::athlete_physiology`).
//!
//! An edit is refused while the athlete's tenant has `set_physiology`
//! switched off ([`refuse_when_edits_off`]): the edits save through the
//! tool's writer, so the tool's switch stops them too.
//!
//! A speed is entered as the screen shows it: as seconds of pace per
//! kilometre or mile on the run and per 100 m in the pool ([`PaceDistance`]),
//! converted here into the metres per second every anchor stores.
//!
//! The writer is built by the `tools-config` feature, and so is this module:
//! without it neither edit route exists, and the `GET`s still read.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_core::constants::units::METERS_PER_KM;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::periodization::SportFamily;
use pierre_core::models::{AnchorOrigin, AnchorUnit, SportAnchorName, TenantId, UnitSystem};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_tool_runtime::guardian::tenant_tool_enabled;
use pierre_tool_runtime::implementations::physiology::save_physiology;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{physiology_response, PhysiologyQuery, PhysiologyResponse};
use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::active_tenant;
use crate::tools::runtime_adapter::into_runtime;

/// The distance an entered pace is in seconds per.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum PaceDistance {
    /// A kilometre: a run pace for a metric reader.
    #[serde(rename = "km")]
    Kilometre,
    /// A mile: a run pace for an imperial reader.
    #[serde(rename = "mi")]
    Mile,
    /// 100 metres: a pool pace, in either unit system.
    #[serde(rename = "100m")]
    HundredMetres,
}

/// The metres in 100 m, the distance a pool pace is written per.
const SWIM_PACE_METRES: f64 = 100.0;

impl PaceDistance {
    /// The metres in this distance: a kilometre or a mile as the formatter
    /// counts them, so an entered pace reads back as it was typed.
    fn metres(self) -> f64 {
        match self {
            Self::Kilometre => METERS_PER_KM / UnitSystem::Metric.distance_from_km(1.0),
            Self::Mile => METERS_PER_KM / UnitSystem::Imperial.distance_from_km(1.0),
            Self::HundredMetres => SWIM_PACE_METRES,
        }
    }

    /// Whether a pace in `sport` is entered per this distance, as the screen
    /// shows it: per 100 m in the pool, per kilometre or mile on the run.
    const fn entered_in(self, sport: SportFamily) -> bool {
        match self {
            Self::HundredMetres => matches!(sport, SportFamily::Swim),
            Self::Kilometre | Self::Mile => matches!(sport, SportFamily::Run),
        }
    }
}

/// Body of a `PUT` setting one anchor: the athlete's own
/// (`/api/me/physiology/{sport}/{anchor}`) or their coach's
/// (`/api/chat/athletes/{id}/physiology/{sport}/{anchor}`).
///
/// `value`, `low` and `high` are in the anchor's unit — watts or beats per
/// minute — or, for a speed anchor, in seconds of pace per `pace_per`. The
/// body names no source: who set the anchor is the route's to say. `kind`,
/// `confidence`, `method` and `measured_on` are read by the physiology
/// writer, as `set_physiology` reads them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorEdit {
    /// The value.
    value: f64,
    /// The low end of the range it stands for: for a pace, the fewer seconds
    /// (the faster end).
    #[serde(default)]
    low: Option<f64>,
    /// The high end of the range it stands for: for a pace, the more seconds
    /// (the slower end).
    #[serde(default)]
    high: Option<f64>,
    /// For a speed anchor, the distance `value`, `low` and `high` are seconds
    /// per; refused on any other anchor.
    #[serde(default)]
    pace_per: Option<PaceDistance>,
    /// `measured` when a named test produced it, `estimated` otherwise.
    kind: String,
    /// `low`, `medium` or `high`, as the editor states it; low when absent.
    #[serde(default)]
    confidence: Option<String>,
    /// The test used; for MAP, its definition.
    #[serde(default)]
    method: Option<String>,
    /// The day it was measured, `YYYY-MM-DD`; today when absent.
    #[serde(default)]
    measured_on: Option<String>,
}

/// The sport family and anchor a `PUT` path names.
///
/// # Errors
///
/// Returns an invalid-input error naming the families or anchors there are,
/// for a name that is neither.
pub(crate) fn anchor_in_path(
    sport: &str,
    anchor: &str,
) -> AppResult<(SportFamily, SportAnchorName)> {
    let family = SportFamily::ALL
        .iter()
        .copied()
        .find(|family| family.as_str() == sport)
        .ok_or_else(|| {
            AppError::invalid_input(format!("sport must be ride, run or swim; got '{sport}'"))
        })?;
    let name = SportAnchorName::parse(anchor).ok_or_else(|| {
        AppError::invalid_input(format!(
            "anchor must be one of {}; got '{anchor}'",
            SportAnchorName::ALL.map(SportAnchorName::as_str).join(", ")
        ))
    })?;
    Ok((family, name))
}

/// `edit` as the `set_physiology` arguments that state it: one
/// `sport_anchors` entry for `sport`'s `anchor`, its source `source`.
///
/// A pace is converted to the metres per second the anchor stores; a slower
/// pace is a lower speed, so the range's ends trade places. Every other
/// check — the anchor offered in the sport, its bounds, a range that holds
/// its value, the kind, the confidence, the method, the day — is the
/// writer's.
///
/// # Errors
///
/// Returns an invalid-input error for a speed anchor without `pace_per`, a
/// `pace_per` the sport is not entered in or on an anchor that is no speed,
/// and a pace that is not a positive number of seconds.
pub(crate) fn anchor_edit_args(
    sport: SportFamily,
    anchor: SportAnchorName,
    edit: &AnchorEdit,
    source: &str,
) -> AppResult<Value> {
    let (value, low, high) = match (anchor.unit(), edit.pace_per) {
        // An anchor the sport cannot hold goes to the writer as typed, which
        // refuses it in its own words (no speed on the bike, no power in the
        // pool) rather than here as a pace entered in the wrong distance.
        _ if !anchor.offered_in(sport) => (edit.value, edit.low, edit.high),
        (AnchorUnit::MetresPerSecond, Some(per)) => {
            if !per.entered_in(sport) {
                return Err(AppError::invalid_input(format!(
                    "a {sport} pace is entered per {}",
                    if matches!(sport, SportFamily::Swim) {
                        "100m"
                    } else {
                        "km or mi"
                    }
                )));
            }
            let metres = per.metres();
            let speed = |seconds: f64| {
                if seconds.is_finite() && seconds > 0.0 {
                    Ok(metres / seconds)
                } else {
                    Err(AppError::invalid_input(format!(
                        "a pace must be a positive number of seconds, got {seconds}"
                    )))
                }
            };
            (
                speed(edit.value)?,
                edit.high.map(speed).transpose()?,
                edit.low.map(speed).transpose()?,
            )
        }
        (AnchorUnit::MetresPerSecond, None) => {
            return Err(AppError::invalid_input(format!(
                "{} is entered as a pace: pace_per is km or mi on the run, 100m in the pool",
                anchor.as_str()
            )));
        }
        (_, Some(_)) => {
            return Err(AppError::invalid_input(format!(
                "pace_per applies to a speed anchor, and {} is none",
                anchor.as_str()
            )));
        }
        (_, None) => (edit.value, edit.low, edit.high),
    };
    Ok(json!({
        "sport_anchors": [{
            "sport": sport.as_str(),
            "anchor": anchor.as_str(),
            "value": value,
            "low": low,
            "high": high,
            "kind": edit.kind,
            "confidence": edit.confidence,
            "method": edit.method,
            "measured_on": edit.measured_on,
            "source": source,
        }]
    }))
}

/// `PUT /api/me/physiology/{sport}/{anchor}`: the athlete sets one of their
/// own anchors, stored as theirs, and reads their anchors back.
///
/// 400 for a sport or anchor the path does not name, and for every edit the
/// writer refuses: out of bounds, a range that does not hold its value, a
/// kind or confidence it does not know, a future day, an anchor the sport
/// cannot hold, or a value that contradicts a stored one. 403
/// `physiology_disabled` when the athlete's tenant switched `set_physiology`
/// off.
pub(in super::super) async fn put_physiology_anchor(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((sport, anchor)): Path<(String, String)>,
    Query(query): Query<PhysiologyQuery>,
    Json(edit): Json<AnchorEdit>,
) -> AppResult<Json<PhysiologyResponse>> {
    let tenant_id = active_tenant(&auth)?;
    let (sport, anchor) = anchor_in_path(&sport, &anchor)?;
    refuse_when_edits_off(&resources, tenant_id, AnchorReader::Athlete).await?;
    let args = anchor_edit_args(sport, anchor, &edit, AnchorOrigin::Athlete.as_str())?;
    save_physiology(&into_runtime(&resources), tenant_id, auth.user_id, &args).await?;
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

/// The tool a tenant switches off to stop every write of its athletes'
/// physiology: the agent's, the athlete's own edit, and their coach's.
const PHYSIOLOGY_WRITE_TOOL: &str = "set_physiology";

/// The `details.reason` of an edit refused because the athlete's tenant
/// switched [`PHYSIOLOGY_WRITE_TOOL`] off.
const PHYSIOLOGY_DISABLED: &str = "physiology_disabled";

/// Refuse an edit of the anchors of an athlete whose tenant `tenant` has
/// `set_physiology` switched off, as the coach's plan read refuses when the
/// tenant switched `get_training_plan` off: through
/// [`tenant_tool_enabled`], with a 403 naming its reason. The edits save
/// through the tool's own writer, so the tool's switch is theirs too.
///
/// Both clients show a 403's message as it stands, so it is worded for
/// `reader`: the athlete reads of their own workspace, the coach of the
/// athlete's.
///
/// # Errors
///
/// Returns the 403 `physiology_disabled` refusal when the switch is off.
pub(crate) async fn refuse_when_edits_off(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    reader: AnchorReader,
) -> AppResult<()> {
    if tenant_tool_enabled(&into_runtime(resources), tenant, PHYSIOLOGY_WRITE_TOOL).await {
        return Ok(());
    }
    let mut error = match reader {
        AnchorReader::Athlete => AppError::new(
            ErrorCode::PermissionDenied,
            "Editing thresholds is turned off in your workspace",
        ),
        AnchorReader::Coach => AppError::new(
            ErrorCode::PermissionDenied,
            "Editing thresholds is turned off in this athlete's workspace",
        ),
    };
    error.details = Some(Box::new(json!({ "reason": PHYSIOLOGY_DISABLED })));
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(body: Value) -> AnchorEdit {
        serde_json::from_value(body).expect("fixture: a well-formed edit")
    }

    fn entry(args: &Value) -> &Value {
        &args["sport_anchors"][0]
    }

    fn close(actual: &Value, expected: f64) -> bool {
        actual.as_f64().is_some_and(|v| (v - expected).abs() < 1e-9)
    }

    #[test]
    fn a_run_pace_per_kilometre_or_mile_becomes_the_speed_it_is() {
        let km = anchor_edit_args(
            SportFamily::Run,
            SportAnchorName::ThresholdSpeed,
            &edit(json!({ "value": 250, "pace_per": "km", "kind": "measured" })),
            "athlete",
        )
        .expect("a run pace per km is accepted");
        assert!(
            close(&entry(&km)["value"], 4.0),
            "bug: 4:10/km is 1000 m in 250 s, 4 m/s: {km}"
        );
        let mile = anchor_edit_args(
            SportFamily::Run,
            SportAnchorName::ThresholdSpeed,
            &edit(json!({ "value": 402.336, "pace_per": "mi", "kind": "measured" })),
            "athlete",
        )
        .expect("a run pace per mile is accepted");
        assert!(
            close(&entry(&mile)["value"], 4.0),
            "bug: 402.336 s per mile is 1609.344 m in 402.336 s, 4 m/s: {mile}"
        );
    }

    #[test]
    fn a_pool_pace_is_per_100_metres_and_a_paces_range_turns_over() {
        let args = anchor_edit_args(
            SportFamily::Swim,
            SportAnchorName::CriticalSpeed,
            &edit(json!({ "value": 80, "low": 78, "high": 82, "pace_per": "100m", "kind": "estimated" })),
            "coach",
        )
        .expect("a pool pace per 100 m is accepted");
        let stated = entry(&args);
        assert!(
            close(&stated["value"], 1.25),
            "bug: 1:20/100 m is 1.25 m/s: {args}"
        );
        assert!(
            close(&stated["high"], 100.0 / 78.0),
            "bug: the faster pace end is the higher speed: {args}"
        );
        assert!(
            close(&stated["low"], 100.0 / 82.0),
            "bug: the slower pace end is the lower speed: {args}"
        );
        assert_eq!(stated["source"], "coach", "bug: the route says who set it");
    }

    #[test]
    fn a_power_or_heart_rate_passes_as_entered_and_names_no_pace() {
        let args = anchor_edit_args(
            SportFamily::Ride,
            SportAnchorName::Ftp,
            &edit(json!({ "value": 285, "low": 280, "high": 290, "kind": "measured", "confidence": "high" })),
            "athlete",
        )
        .expect("watts pass through");
        assert_eq!(
            entry(&args),
            &json!({
                "sport": "ride", "anchor": "ftp", "value": 285.0, "low": 280.0, "high": 290.0,
                "kind": "measured", "confidence": "high", "method": null, "measured_on": null,
                "source": "athlete"
            }),
            "bug: an edit is the writer's own sport_anchors entry"
        );
        let refused = anchor_edit_args(
            SportFamily::Ride,
            SportAnchorName::Ftp,
            &edit(json!({ "value": 285, "pace_per": "km", "kind": "measured" })),
            "athlete",
        );
        assert!(refused.is_err(), "bug: a pace on a power anchor is refused");
    }

    #[test]
    fn a_speed_names_its_pace_distance_and_the_sport_it_is_entered_in() {
        let missing = anchor_edit_args(
            SportFamily::Run,
            SportAnchorName::CriticalSpeed,
            &edit(json!({ "value": 250, "kind": "measured" })),
            "athlete",
        );
        assert!(
            missing.is_err(),
            "bug: a speed with no pace distance is refused"
        );
        let wrong = anchor_edit_args(
            SportFamily::Swim,
            SportAnchorName::CriticalSpeed,
            &edit(json!({ "value": 80, "pace_per": "km", "kind": "measured" })),
            "athlete",
        );
        assert!(
            wrong.is_err(),
            "bug: a pool pace is entered per 100 m, never per km"
        );
        let zero = anchor_edit_args(
            SportFamily::Run,
            SportAnchorName::CriticalSpeed,
            &edit(json!({ "value": 0, "pace_per": "km", "kind": "measured" })),
            "athlete",
        );
        assert!(zero.is_err(), "bug: a zero pace has no speed");
    }

    #[test]
    fn an_edit_naming_its_own_source_is_refused() {
        let refused = serde_json::from_value::<AnchorEdit>(
            json!({ "value": 285, "kind": "measured", "source": "intervals.icu" }),
        );
        assert!(
            refused.is_err(),
            "bug: who set an anchor is the route's to say, never the body's"
        );
    }
}
