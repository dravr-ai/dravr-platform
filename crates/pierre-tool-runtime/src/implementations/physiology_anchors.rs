// ABOUTME: set_physiology's per-sport anchors — parse and range-check each statement, mirror a twin onto the profile row
// ABOUTME: A profile field and the per-sport anchor it mirrors never disagree after a save: whichever the call states wins
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `sport_anchors` argument of `set_physiology` (carnet#1496).
//!
//! An anchor is stated per sport family with its value, the range it stands
//! for, whether a named test produced it, who produced it, on which day, by
//! which method (for MAP, the definition it follows) and how far to trust it.
//! Six anchors also live on the profile row that training load, zones and the
//! provider imports read ([`ProfileTwin`]): the bike's FTP and critical power,
//! the run's critical speed and threshold pace, and the main sport's maximum
//! and threshold heart rates. A stated twin anchor is mirrored onto its field,
//! and a plain field the call sets supersedes its stored twin anchor, so the
//! two never disagree; a call stating both for one twin is refused.

use std::collections::BTreeSet;

use chrono::{NaiveDate, Utc};
use dravr_cageux::physiological_constants::configuration_validation;
use pierre_core::constants::units::METERS_PER_KM;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{
    AnchorConfidence, AnchorValue, AthleteAnchors, SportFamily,
};
use pierre_core::models::{
    sport_anchors, AnchorOrigin, AnchorUnit, MeasurementKind, MetricProvenance, ProfileTwin,
    ProvenancedValue, SportAnchor, SportAnchorName, UserPhysiologicalProfile,
};
use serde_json::Value;

use super::physiology::{
    CRITICAL_POWER_WATTS_MAX, CRITICAL_POWER_WATTS_MIN, CRITICAL_SPEED_MPS_MAX,
    CRITICAL_SPEED_MPS_MIN, MEASUREMENT_SOURCE_MAX_CHARS, THRESHOLD_PACE_SEC_PER_KM_MAX,
    THRESHOLD_PACE_SEC_PER_KM_MIN,
};

/// The argument name.
pub(crate) const SPORT_ANCHORS_ARG: &str = "sport_anchors";

/// Highest maximal aerobic power accepted, in watts: the critical-power
/// ceiling times the top of the MAP-to-CP spread (cageux's I8 default is
/// 1.30), rounded up.
const MAP_WATTS_MAX: f64 = 800.0;

/// Fastest run `vVO2max` or MAS accepted, in metres per second: the
/// critical-speed ceiling (7.0 m/s) times the top of the `vVO2max`-to-CS
/// spread (1.16, cageux I8), rounded up.
const RUN_AEROBIC_SPEED_MPS_MAX: f64 = 8.5;

/// Slowest swim speed accepted for any swim anchor, in metres per second
/// (4:10 per 100 m).
const SWIM_SPEED_MPS_MIN: f64 = 0.4;

/// Fastest swim speed accepted for any swim anchor, in metres per second:
/// below the 50 m freestyle world-record average (about 2.39 m/s), above any
/// aerobic swimming speed.
const SWIM_SPEED_MPS_MAX: f64 = 2.2;

/// One anchor as the call states it, before it is dated.
#[derive(Debug, Clone)]
struct AnchorStatement {
    sport: SportFamily,
    anchor: SportAnchorName,
    value: f64,
    low: Option<f64>,
    high: Option<f64>,
    kind: MeasurementKind,
    origin: String,
    measured_on: Option<NaiveDate>,
    method: Option<String>,
    confidence: AnchorConfidence,
}

/// The `sport_anchors` one call carries; empty when it carries none.
#[derive(Debug, Default)]
pub(crate) struct SportAnchorUpdate {
    statements: Vec<AnchorStatement>,
}

impl SportAnchorUpdate {
    /// Parse and range-check `sport_anchors`.
    ///
    /// # Errors
    /// Returns an invalid-input error naming the entry when one is not an
    /// object, names a sport or anchor the tool does not know, offers an
    /// anchor in a sport that cannot hold it (power in the pool, speed on the
    /// bike), carries a value or range outside the anchor's bounds or a range
    /// that does not hold its value, lacks its kind, carries an overlong
    /// method or source or a future day, or repeats an earlier entry.
    pub(crate) fn from_args(args: &Value) -> AppResult<Self> {
        let Some(raw) = args.get(SPORT_ANCHORS_ARG).filter(|v| !v.is_null()) else {
            return Ok(Self::default());
        };
        let items = raw.as_array().ok_or_else(|| {
            AppError::invalid_input("sport_anchors must be an array of anchor objects")
        })?;
        let mut seen = BTreeSet::new();
        let mut statements = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let statement = AnchorStatement::parse(item)
                .map_err(|e| AppError::invalid_input(format!("sport_anchors[{index}]: {e}")))?;
            if !seen.insert((statement.sport, statement.anchor)) {
                return Err(AppError::invalid_input(format!(
                    "sport_anchors[{index}] repeats the {} {} anchor; state each once",
                    statement.sport,
                    statement.anchor.as_str()
                )));
            }
            statements.push(statement);
        }
        Ok(Self { statements })
    }

    /// Whether the call states no anchor.
    pub(crate) const fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

    /// The stated anchors as stored, each dated `today` when the call named
    /// no day.
    pub(crate) fn anchors(&self, today: NaiveDate) -> Vec<SportAnchor> {
        self.statements
            .iter()
            .map(|s| {
                // Range-checked against bounds far inside f32.
                #[allow(clippy::cast_possible_truncation)]
                let narrow = |v: f64| v as f32;
                SportAnchor {
                    sport: s.sport,
                    anchor: s.anchor,
                    kind: s.kind,
                    value: AnchorValue {
                        value: narrow(s.value),
                        low: s.low.map(narrow),
                        high: s.high.map(narrow),
                        origin: Some(s.origin.clone()),
                        as_of: Some(s.measured_on.unwrap_or(today)),
                        method: s.method.clone(),
                        confidence: s.confidence,
                    },
                }
            })
            .collect()
    }
}

impl AnchorStatement {
    /// Parse one entry. The error is the reason alone; the caller names the
    /// entry.
    fn parse(item: &Value) -> Result<Self, String> {
        let object = item
            .as_object()
            .ok_or_else(|| "each anchor must be an object".to_owned())?;
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::trim);
        let sport_raw = text("sport").ok_or_else(|| "sport is required".to_owned())?;
        let sport = SportFamily::ALL
            .iter()
            .copied()
            .find(|family| family.as_str() == sport_raw.to_ascii_lowercase())
            .ok_or_else(|| format!("sport must be ride, run or swim; got '{sport_raw}'"))?;
        let anchor_raw = text("anchor").ok_or_else(|| "anchor is required".to_owned())?;
        let anchor = SportAnchorName::parse(&anchor_raw.to_ascii_lowercase()).ok_or_else(|| {
            format!(
                "anchor must be one of {}; got '{anchor_raw}'",
                SportAnchorName::ALL.map(SportAnchorName::as_str).join(", ")
            )
        })?;
        if !anchor.offered_in(sport) {
            return Err(match anchor.unit() {
                AnchorUnit::Watts => {
                    format!("{} is a power anchor, and swim has none", anchor.as_str())
                }
                _ => format!("{} is a speed anchor, and ride has none", anchor.as_str()),
            });
        }
        let number = |key: &str| -> Result<Option<f64>, String> {
            match object.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("{key} must be a number")),
            }
        };
        let value = number("value")?.ok_or_else(|| "value is required".to_owned())?;
        let low = number("low")?;
        let high = number("high")?;
        let (min, max) = bounds(sport, anchor);
        for (name, candidate) in [("value", Some(value)), ("low", low), ("high", high)] {
            if let Some(v) = candidate {
                if !(min..=max).contains(&v) {
                    return Err(format!(
                        "{name} for the {sport} {} must be between {min} and {max} {}, got {v}",
                        anchor.as_str(),
                        unit_label(anchor.unit())
                    ));
                }
            }
        }
        if low.is_some_and(|l| l > value) || high.is_some_and(|h| h < value) {
            return Err("low and high must hold the value between them".to_owned());
        }
        let kind = match text("kind").map(str::to_ascii_lowercase).as_deref() {
            Some("measured") => MeasurementKind::Measured,
            Some("estimated") => MeasurementKind::Estimated,
            Some(other) => return Err(format!("kind must be measured or estimated; got '{other}'")),
            None => {
                return Err(
                    "kind is required: measured only when a named test produced it, estimated otherwise"
                        .to_owned(),
                )
            }
        };
        let confidence = match text("confidence") {
            None => AnchorConfidence::Low,
            Some(raw) => AnchorConfidence::ALL
                .iter()
                .copied()
                .find(|level| level.as_str() == raw.to_ascii_lowercase())
                .ok_or_else(|| format!("confidence must be low, medium or high; got '{raw}'"))?,
        };
        let bounded = |key: &str| -> Result<Option<String>, String> {
            let Some(v) = text(key).filter(|s| !s.is_empty()) else {
                return Ok(None);
            };
            if v.chars().count() > MEASUREMENT_SOURCE_MAX_CHARS {
                return Err(format!(
                    "{key} must be at most {MEASUREMENT_SOURCE_MAX_CHARS} characters"
                ));
            }
            Ok(Some(v.to_owned()))
        };
        let method = bounded("method")?;
        let origin =
            bounded("source")?.unwrap_or_else(|| AnchorOrigin::Athlete.as_str().to_owned());
        let measured_on = text("measured_on")
            .map(|raw| {
                NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                    .map_err(|_| format!("measured_on must be a date as YYYY-MM-DD; got '{raw}'"))
            })
            .transpose()?;
        if let Some(day) = measured_on {
            if day > Utc::now().date_naive() {
                return Err(format!("measured_on cannot be in the future; got {day}"));
            }
        }
        Ok(Self {
            sport,
            anchor,
            value,
            low,
            high,
            kind,
            origin,
            measured_on,
            method,
            confidence,
        })
    }
}

/// The inclusive bounds an anchor's value and range must sit in, in its unit.
fn bounds(sport: SportFamily, anchor: SportAnchorName) -> (f64, f64) {
    let swim = matches!(sport, SportFamily::Swim);
    // Every bound is a small whole number of bpm or watts.
    #[allow(clippy::cast_precision_loss)]
    match anchor {
        SportAnchorName::Ftp => (
            configuration_validation::FTP_MIN as f64,
            configuration_validation::FTP_MAX as f64,
        ),
        SportAnchorName::CriticalPower => (
            f64::from(CRITICAL_POWER_WATTS_MIN),
            f64::from(CRITICAL_POWER_WATTS_MAX),
        ),
        SportAnchorName::Map => (f64::from(CRITICAL_POWER_WATTS_MIN), MAP_WATTS_MAX),
        _ if swim => (SWIM_SPEED_MPS_MIN, SWIM_SPEED_MPS_MAX),
        SportAnchorName::CriticalSpeed => (CRITICAL_SPEED_MPS_MIN, CRITICAL_SPEED_MPS_MAX),
        SportAnchorName::ThresholdSpeed => (
            METERS_PER_KM / THRESHOLD_PACE_SEC_PER_KM_MAX,
            METERS_PER_KM / THRESHOLD_PACE_SEC_PER_KM_MIN,
        ),
        SportAnchorName::Vvo2max => (CRITICAL_SPEED_MPS_MIN, RUN_AEROBIC_SPEED_MPS_MAX),
        SportAnchorName::MaxHeartRate => (
            configuration_validation::MAX_HR_MIN as f64,
            configuration_validation::MAX_HR_MAX as f64,
        ),
        SportAnchorName::ThresholdHeartRate => (
            configuration_validation::THRESHOLD_HR_MIN as f64,
            configuration_validation::THRESHOLD_HR_MAX as f64,
        ),
    }
}

/// The unit's short label, for error messages.
const fn unit_label(unit: AnchorUnit) -> &'static str {
    match unit {
        AnchorUnit::Watts => "W",
        AnchorUnit::MetresPerSecond => "m/s",
        AnchorUnit::BeatsPerMinute => "bpm",
    }
}

/// Write a stated twin anchor onto the profile field it mirrors, so training
/// load, zones and imports read the same number the workout families do.
///
/// A scalar threshold becomes the athlete's own, dated by the anchor, as a
/// plain field write does; critical power and speed keep the anchor's kind,
/// origin and day.
fn mirror_onto(profile: &mut UserPhysiologicalProfile, twin: ProfileTwin, anchor: &SportAnchor) {
    let value = f64::from(anchor.value.value);
    let as_of = anchor.value.as_of;
    let provenance = || MetricProvenance {
        kind: anchor.kind,
        origin: anchor.value.origin.clone(),
        as_of,
    };
    // Range-checked: watts at most 800 and bpm at most 220.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = |v: f64| v.round() as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bpm = |v: f64| v.round() as u16;
    match twin {
        ProfileTwin::FtpWatts => {
            profile.ftp_watts = Some(whole(value));
            profile.threshold_provenance.ftp_watts = None;
            profile.threshold_stated_on.ftp_watts = as_of;
        }
        ProfileTwin::CriticalPowerWatts => {
            profile.critical_power_watts = Some(ProvenancedValue::new(whole(value), provenance()));
        }
        ProfileTwin::CriticalSpeedMps => {
            profile.critical_speed_mps = Some(ProvenancedValue::new(value, provenance()));
        }
        ProfileTwin::ThresholdPaceSecPerKm => {
            profile.threshold_pace_sec_per_km = Some(METERS_PER_KM / value);
            profile.threshold_provenance.threshold_pace_sec_per_km = None;
            profile.threshold_stated_on.threshold_pace_sec_per_km = as_of;
        }
        ProfileTwin::MaxHr => {
            profile.max_hr = Some(bpm(value));
            profile.threshold_provenance.max_hr = None;
            profile.threshold_stated_on.max_hr = as_of;
        }
        ProfileTwin::ThresholdHr => {
            profile.threshold_hr = Some(bpm(value));
            profile.threshold_provenance.threshold_hr = None;
            profile.threshold_stated_on.threshold_hr = as_of;
        }
    }
}

/// What one call writes to the anchor table once the profile row is saved:
/// the anchor table's rows need their profile row beside them.
#[derive(Debug, Default)]
pub(crate) struct AnchorWrites {
    /// Anchors to write or replace.
    pub(crate) upserts: Vec<SportAnchor>,
    /// Stored twin anchors a plain field this call sets supersedes.
    pub(crate) deletes: Vec<(SportFamily, SportAnchorName)>,
    /// Every per-sport anchor as it will stand after the writes.
    pub(crate) merged: Vec<SportAnchor>,
}

/// Reconcile the call's plain fields and stated anchors with what is stored.
///
/// `profile` already carries the plain fields (`plain_fields` names them).
/// Each stated twin anchor is mirrored onto its field; each plain twin field
/// supersedes its stored anchor, which is deleted, since the anchor's range
/// and confidence described the number the athlete just replaced. The merged
/// anchors are checked by [`check_relationships`] once the profile row has
/// passed its own validation.
///
/// # Errors
/// Returns an invalid-input error when the call states one twin both as a
/// field and as an anchor.
pub(crate) fn reconcile(
    profile: &mut UserPhysiologicalProfile,
    plain_fields: &[&str],
    stated: Vec<SportAnchor>,
    stored: &[SportAnchor],
) -> AppResult<AnchorWrites> {
    let primary = SportFamily::of(&profile.primary_sport);
    let same = |a: &SportAnchor, sport: SportFamily, name: SportAnchorName| {
        a.sport == sport && a.anchor == name
    };
    let mut deletes = Vec::new();
    for twin in ProfileTwin::ALL {
        if !plain_fields.contains(&twin.field()) {
            continue;
        }
        let Some((sport, name)) = twin.anchor(primary) else {
            continue;
        };
        if stated.iter().any(|a| same(a, sport, name)) {
            return Err(AppError::invalid_input(format!(
                "{} and the {sport} {} anchor are the same value; state it once, in sport_anchors to keep its range and confidence",
                twin.field(),
                name.as_str()
            )));
        }
        if stored.iter().any(|a| same(a, sport, name)) {
            deletes.push((sport, name));
        }
    }
    for anchor in &stated {
        if let Some(twin) = anchor.anchor.profile_twin(anchor.sport, primary) {
            mirror_onto(profile, twin, anchor);
        }
    }
    let merged: Vec<SportAnchor> = stored
        .iter()
        .filter(|a| {
            !deletes.contains(&(a.sport, a.anchor))
                && !stated.iter().any(|s| same(s, a.sport, a.anchor))
        })
        .cloned()
        .chain(stated.iter().cloned())
        .collect();
    Ok(AnchorWrites {
        upserts: stated,
        deletes,
        merged,
    })
}

/// Check the anchors `profile` and `merged` give each sport against the
/// relationships they must keep ([`relationship_errors`]).
///
/// Run after the profile row's own validation, so a contradiction on the row
/// (a threshold heart rate at or above the maximum) is reported in the row's
/// own words, and this catches what only the per-sport anchors hold.
///
/// # Errors
/// Returns an invalid-input error naming every broken relationship.
pub(crate) fn check_relationships(
    profile: &UserPhysiologicalProfile,
    merged: &[SportAnchor],
) -> AppResult<()> {
    let errors = relationship_errors(&sport_anchors::athlete_anchors(profile, merged));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::invalid_input(errors.join("; ")))
    }
}

/// The relationships each sport's merged anchors must keep, as error
/// messages: MAP above critical power and `vVO2max` above critical speed,
/// since the severe domain lies between each pair, and the threshold heart
/// rate below the maximum. Empty when every pair holds.
fn relationship_errors(anchors: &AthleteAnchors) -> Vec<String> {
    let mut errors = Vec::new();
    for sport in SportFamily::ALL {
        let held = anchors.for_family(*sport);
        for (upper, lower) in [
            (SportAnchorName::Map, SportAnchorName::CriticalPower),
            (SportAnchorName::Vvo2max, SportAnchorName::CriticalSpeed),
            (
                SportAnchorName::MaxHeartRate,
                SportAnchorName::ThresholdHeartRate,
            ),
        ] {
            if let (Some(high), Some(low)) = (upper.get(held), lower.get(held)) {
                if high.value <= low.value {
                    errors.push(format!(
                        "the {sport} {} ({}) must be above its {} ({})",
                        upper.as_str(),
                        high.value,
                        lower.as_str(),
                        low.value
                    ));
                }
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use pierre_core::models::SportType;
    use serde_json::json;
    use uuid::Uuid;

    type TestResult = Result<(), Box<dyn Error>>;

    fn parse(entry: &Value) -> AppResult<SportAnchorUpdate> {
        SportAnchorUpdate::from_args(&json!({ "sport_anchors": [entry] }))
    }

    fn refusal(result: AppResult<SportAnchorUpdate>) -> Result<String, Box<dyn Error>> {
        match result {
            Ok(_) => Err("expected a refusal".into()),
            Err(e) => Ok(e.to_string()),
        }
    }

    fn date(year: i32, month: u32, day: u32) -> Result<NaiveDate, Box<dyn Error>> {
        NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| "fixture date".into())
    }

    #[test]
    fn a_full_statement_parses_and_dates_itself_today_when_undated() -> TestResult {
        let update = parse(&json!({
            "sport": "ride", "anchor": "map", "value": 380, "low": 365, "high": 395,
            "kind": "measured", "confidence": "high", "method": "ramp peak", "source": "lab"
        }))?;
        let today = date(2026, 11, 11)?;
        let anchors = update.anchors(today);
        assert_eq!(anchors.len(), 1);
        let map = &anchors[0];
        assert_eq!(map.anchor, SportAnchorName::Map);
        assert_eq!(map.value.as_of, Some(today));
        assert_eq!(map.value.origin.as_deref(), Some("lab"));
        assert_eq!(map.value.method.as_deref(), Some("ramp peak"));
        assert_eq!(map.value.confidence, AnchorConfidence::High);
        Ok(())
    }

    #[test]
    fn an_undeclared_confidence_is_low_and_an_unnamed_source_is_the_athlete() -> TestResult {
        let update = parse(&json!({
            "sport": "run", "anchor": "critical_speed", "value": 4.4, "kind": "estimated"
        }))?;
        let anchors = update.anchors(NaiveDate::MIN);
        let anchor = anchors.first().ok_or("one anchor")?;
        assert_eq!(anchor.value.confidence, AnchorConfidence::Low);
        assert_eq!(anchor.value.origin.as_deref(), Some("athlete"));
        Ok(())
    }

    #[test]
    fn power_in_the_pool_and_speed_on_the_bike_are_refused() -> TestResult {
        assert!(refusal(parse(&json!({
            "sport": "swim", "anchor": "ftp", "value": 250, "kind": "measured"
        })))?
        .contains("swim has none"));
        assert!(refusal(parse(&json!({
            "sport": "ride", "anchor": "vvo2max", "value": 5.0, "kind": "measured"
        })))?
        .contains("ride has none"));
        Ok(())
    }

    #[test]
    fn a_swim_speed_is_held_to_swimming_bounds() -> TestResult {
        assert!(parse(&json!({
            "sport": "swim", "anchor": "critical_speed", "value": 1.25, "kind": "measured"
        }))
        .is_ok());
        assert!(refusal(parse(&json!({
            "sport": "swim", "anchor": "critical_speed", "value": 4.0, "kind": "measured"
        })))?
        .contains("between 0.4 and 2.2"));
        Ok(())
    }

    #[test]
    fn a_range_must_hold_its_value_and_a_kind_is_required() -> TestResult {
        assert!(refusal(parse(&json!({
            "sport": "ride", "anchor": "ftp", "value": 280, "low": 290, "kind": "measured"
        })))?
        .contains("hold the value"));
        assert!(refusal(parse(&json!({
            "sport": "ride", "anchor": "ftp", "value": 280
        })))?
        .contains("kind is required"));
        Ok(())
    }

    #[test]
    fn the_same_anchor_twice_in_one_call_is_refused() -> TestResult {
        let refused = SportAnchorUpdate::from_args(&json!({ "sport_anchors": [
            { "sport": "run", "anchor": "max_heart_rate", "value": 192, "kind": "measured" },
            { "sport": "run", "anchor": "max_heart_rate", "value": 194, "kind": "measured" }
        ]}));
        assert!(refusal(refused)?.contains("state each once"));
        Ok(())
    }

    #[test]
    fn a_mirrored_threshold_becomes_the_athletes_own_and_dated() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.threshold_provenance.ftp_watts = Some(MetricProvenance {
            kind: MeasurementKind::Estimated,
            origin: Some("intervals.icu".to_owned()),
            as_of: None,
        });
        let day = date(2026, 11, 2)?;
        let ftp = parse(&json!({
            "sport": "ride", "anchor": "ftp", "value": 291.6, "kind": "measured",
            "measured_on": "2026-11-02"
        }))?
        .anchors(day);
        mirror_onto(
            &mut profile,
            ProfileTwin::FtpWatts,
            ftp.first().ok_or("ftp")?,
        );
        assert_eq!(profile.ftp_watts, Some(292));
        assert!(profile.threshold_provenance.ftp_watts.is_none());
        assert_eq!(profile.threshold_stated_on.ftp_watts, Some(day));

        let pace = parse(&json!({
            "sport": "run", "anchor": "threshold_speed", "value": 4.0, "kind": "estimated"
        }))?
        .anchors(day);
        mirror_onto(
            &mut profile,
            ProfileTwin::ThresholdPaceSecPerKm,
            pace.first().ok_or("pace")?,
        );
        assert_eq!(profile.threshold_pace_sec_per_km, Some(250.0));
        Ok(())
    }

    #[test]
    fn a_plain_field_and_its_twin_anchor_in_one_call_are_refused() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        let stated = parse(&json!({
            "sport": "ride", "anchor": "ftp", "value": 290, "kind": "measured"
        }))?
        .anchors(date(2026, 11, 2)?);
        let Err(refused) = reconcile(&mut profile, &["ftp_watts"], stated, &[]) else {
            return Err("expected a refusal".into());
        };
        assert!(refused.to_string().contains("state it once"));
        Ok(())
    }

    #[test]
    fn a_plain_field_supersedes_its_stored_twin_and_a_stated_twin_mirrors() -> TestResult {
        let day = date(2026, 11, 2)?;
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        let stored = parse(&json!({
            "sport": "ride", "anchor": "max_heart_rate", "value": 186, "kind": "measured"
        }))?
        .anchors(day);
        let stated = parse(&json!({
            "sport": "ride", "anchor": "critical_power", "value": 300, "kind": "estimated",
            "source": "vekta"
        }))?
        .anchors(day);
        profile.max_hr = Some(190);
        let writes = reconcile(&mut profile, &["max_hr"], stated, &stored)?;
        assert_eq!(
            writes.deletes,
            vec![(SportFamily::Ride, SportAnchorName::MaxHeartRate)]
        );
        assert_eq!(writes.upserts.len(), 1);
        let cp = profile.critical_power_watts.as_ref().ok_or("mirrored CP")?;
        assert_eq!(cp.value, 300);
        assert_eq!(cp.provenance.origin.as_deref(), Some("vekta"));
        assert_eq!(cp.provenance.kind, MeasurementKind::Estimated);
        Ok(())
    }

    #[test]
    fn map_at_or_below_critical_power_is_a_relationship_error() {
        let mut anchors = AthleteAnchors::default();
        let point = |value: f32| AnchorValue {
            value,
            low: None,
            high: None,
            origin: None,
            as_of: None,
            method: None,
            confidence: AnchorConfidence::Low,
        };
        anchors.ride.critical_power = Some(point(300.0));
        anchors.ride.map = Some(point(390.0));
        assert_eq!(relationship_errors(&anchors), Vec::<String>::new());
        anchors.ride.map = Some(point(290.0));
        let errors = relationship_errors(&anchors);
        assert_eq!(errors.len(), 1);
        assert!(errors.iter().all(|e| e.contains("ride map")));
    }
}
