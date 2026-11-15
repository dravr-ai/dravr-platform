// ABOUTME: What the physiology tools echo back — the profile as re-read from storage, with its per-sport anchors
// ABOUTME: Shared by set_physiology and import_provider_thresholds, so both describe the stored profile the same way
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{
    AnchorProvenance, HrZoneSet, PhysiologicalSex, PowerZoneSet, ProvenancedValue, SportAnchor,
    SportType, ThresholdProvenance, ThresholdStatedOn, UserPhysiologicalProfile,
};
use serde::Serialize;

use crate::training_history_compute::HistoryRefresh;

/// Render a profile and its per-sport anchors for the tool response.
pub(crate) fn profile_payload(
    profile: &UserPhysiologicalProfile,
    sport_anchors: Vec<SportAnchor>,
) -> PhysiologyProfile {
    PhysiologyProfile {
        ftp_watts: profile.ftp_watts,
        threshold_pace_sec_per_km: profile.threshold_pace_sec_per_km,
        max_hr: profile.max_hr,
        resting_hr: profile.resting_hr,
        threshold_hr: profile.threshold_hr,
        lactate_threshold_percentage: profile.lactate_threshold_percentage,
        vo2_max: profile.vo2_max,
        weight: profile.weight,
        age: profile.age,
        fitness_level: profile.fitness_level,
        primary_sport: profile.primary_sport.clone(),
        training_experience_years: profile.training_experience_years,
        sex: profile.sex,
        hr_zones: profile.hr_zones,
        power_zones: profile.power_zones,
        critical_power_watts: profile.critical_power_watts.clone(),
        w_prime_joules: profile.w_prime_joules.clone(),
        critical_speed_mps: profile.critical_speed_mps.clone(),
        d_prime_meters: profile.d_prime_meters.clone(),
        threshold_provenance: profile.threshold_provenance.clone(),
        threshold_stated_on: profile.threshold_stated_on,
        anchor_provenance: profile.anchor_provenance,
        sport_anchors,
    }
}

/// The athlete's physiological profile as the tools echo it back.
///
/// Every measurement is optional: a profile is built up over time, and an
/// athlete who has only ever given a resting heart rate has a profile with
/// one field set. Reporting the absent ones as absent rather than as zero is
/// what keeps an agent from reasoning off a fabricated number.
///
/// `fitness_level` and `primary_sport` are NOT optional — they carry their
/// own defaults — which is why they are the enums rather than strings.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PhysiologyProfile {
    /// Functional threshold power, in watts.
    pub ftp_watts: Option<u32>,
    /// Threshold pace, in seconds per kilometre.
    pub threshold_pace_sec_per_km: Option<f64>,
    /// Maximum heart rate, in bpm.
    pub max_hr: Option<u16>,
    /// Resting heart rate, in bpm.
    pub resting_hr: Option<u16>,
    /// Measured lactate threshold heart rate, in bpm — what heart-rate
    /// training stress is scored against.
    pub threshold_hr: Option<u16>,
    /// Lactate threshold as a fraction of `VO2max`. Sets threshold pace, and
    /// estimates the LTHR when none is measured.
    pub lactate_threshold_percentage: Option<f64>,
    /// `VO2max`, in ml/kg/min.
    pub vo2_max: Option<f64>,
    /// Body mass, in kilograms.
    pub weight: Option<f64>,
    /// Age in years.
    pub age: Option<u16>,
    /// Self-reported level: beginner, intermediate, advanced.
    pub fitness_level: FitnessLevel,
    /// The sport the athlete mostly trains. Open-ended: a provider-specific
    /// sport no enum anticipated arrives as `Other`.
    pub primary_sport: SportType,
    /// Years of structured training.
    pub training_experience_years: Option<u8>,
    /// Physiological sex, as the athlete stated it or a provider reported it;
    /// absent when neither did. It selects the published equations that take
    /// one.
    pub sex: Option<PhysiologicalSex>,
    /// Heart-rate zone boundaries, cageux's `VO2max`-tiered Karvonen zones
    /// (zone 6 when the tier has one). Absent until both a resting and a
    /// maximum heart rate are on the profile, because they are cut from the
    /// reserve between the pair.
    pub hr_zones: Option<HrZoneSet>,
    /// Power zone boundaries, Coggan's seven levels from the FTP (zone 7 is
    /// everything above `z6_max`). Absent for an athlete with no saved FTP.
    pub power_zones: Option<PowerZoneSet>,
    /// Critical power, in watts, with whether it was measured or estimated
    /// and by whom. Quote an estimated value as an estimate, attributed to
    /// its origin ("Vekta estimates your CP at 312 W"), never as a
    /// measurement.
    pub critical_power_watts: Option<ProvenancedValue<u32>>,
    /// W′, in joules, with its provenance. Quoted the same way as
    /// `critical_power_watts`.
    pub w_prime_joules: Option<ProvenancedValue<u32>>,
    /// Critical speed, in metres per second, with its provenance. Quoted the
    /// same way as `critical_power_watts`.
    pub critical_speed_mps: Option<ProvenancedValue<f64>>,
    /// D′, in metres, with its provenance. Quoted the same way as
    /// `critical_power_watts`.
    pub d_prime_meters: Option<ProvenancedValue<f64>>,
    /// Where `ftp_watts`, `threshold_hr`, `max_hr` and
    /// `threshold_pace_sec_per_km` came from, for the ones a provider supplied
    /// (`import_provider_thresholds`). Each listed value is that provider's
    /// estimate: quote it with its origin ("intervals.icu has your FTP at
    /// 285 W"), never as a measurement. A threshold not listed is the
    /// athlete's own, and the field is absent when all of them are.
    #[serde(skip_serializing_if = "ThresholdProvenance::is_empty")]
    pub threshold_provenance: ThresholdProvenance,
    /// The day the athlete stated each of those four thresholds, for the ones
    /// that are their own and were stated since the day was recorded. Absent
    /// when none is.
    #[serde(skip_serializing_if = "ThresholdStatedOn::is_empty")]
    pub threshold_stated_on: ThresholdStatedOn,
    /// Where `resting_hr`, `vo2_max` and `weight` came from, for the ones with
    /// a recorded origin. `promoted` is filled weekly from the athlete's own
    /// device readings (resting heart rate: the 7-day median of morning
    /// readings): say it came from their devices, not that they set it.
    /// Absent when none is recorded.
    #[serde(skip_serializing_if = "AnchorProvenance::is_empty")]
    pub anchor_provenance: AnchorProvenance,
    /// The per-sport anchors stored, each with the range it stands for, kind,
    /// origin, day, method (for `map`, its definition) and confidence. Quote
    /// a low-confidence anchor as a range or an estimate, never as one precise
    /// number, and an estimated one with its source. Absent when none is
    /// stored.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sport_anchors: Vec<SportAnchor>,
}

/// What `set_physiology` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SetPhysiologyResult {
    /// Always true: the tool errors rather than reporting a failed save.
    pub saved: bool,
    /// Whether this call created the profile rather than updating one.
    pub created: bool,
    /// Which fields this call set, by name. Names only — the measurements
    /// themselves are health data and do not belong in a field list.
    pub updated_fields: Vec<&'static str>,
    /// The profile as it now stands, so the agent need not read it back.
    pub profile: PhysiologyProfile,
    /// What the save did to the stored daily training history
    /// (`get_training_history`): recomputed when it moved a number training
    /// load is scored against.
    pub training_history: HistoryRefresh,
}
