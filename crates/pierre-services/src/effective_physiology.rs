// ABOUTME: The athlete's effective training anchors: the profile's value, else their latest measurement
// ABOUTME: Resting heart rate, VO2max and weight a device measured fill only the anchors the profile leaves empty

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Effective physiology (Athlete Data Consolidation §5, S9)
//!
//! The profile's resting heart rate, `VO2max` and weight are **training
//! anchors**: the numbers zones are cut from and stored training load is
//! scored against. A device measures the same three every day, and those
//! measurements live in the wellness rows (`recovery_metrics`,
//! `health_snapshots`), each source's under its own key — a reading the
//! athlete entered by hand under `athlete`.
//!
//! A reader that only describes the athlete (the personalized-physiology
//! snapshot, the `VO2max` estimate's defaults) reads [`EffectivePhysiology`]:
//! the profile's anchor when it holds one, else the latest measurement of
//! it, read through the one wellness read
//! ([`stored_health::latest_anchor_measurements`]). An anchor the profile
//! holds always wins, however much newer a measurement is: the athlete's
//! own number is never replaced by a device's.
//!
//! A resting heart rate or weight the athlete states with `set_physiology`
//! is both: it sets the anchor, and [`record_manual_measurements`] files it
//! as that day's measurement under the `athlete` key, so trends and merged
//! reads see it, ranked above every device's copy of the same day for the
//! readings it sets. A `VO2max` only sets the anchor: the number handed to
//! `set_physiology` is as often an estimate (`estimate_vo2max`, a race's
//! VDOT) as a lab value, and an estimate is never stored as a measurement.
//!
//! A statement made in a turn that read first-party-only data (the athlete
//! saving the resting heart rate WHOOP showed them) is not filed: a wellness
//! row carries no transport stamp, and the `athlete` key is served over every
//! transport, so the row would carry the number past the stamp the profile
//! holds (carnet#769).
//!
//! What scores stored history keeps reading the stored profile alone
//! (`AthleteInputs::from_profile`): filling it from a daily measurement
//! would recut the zones and every stored day's load each time a watch
//! reported a new resting heart rate.

use chrono::Utc;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::constants::oauth_providers::ATHLETE;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    DataSource, DeviceType, StoredHealthMetrics, StoredRecoveryMetrics, TenantId,
    UserPhysiologicalProfile,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use tracing::{info, warn};
use uuid::Uuid;

use crate::athlete_clock::athlete_zone;
use crate::stored_health::{self, AnchorMeasurements, Anchors};
use crate::wellness_store::{WellnessStore, WellnessWriteMode};

/// The athlete's stored profile with their latest measurement of each
/// anchor it might lack.
#[derive(Debug, Clone, Default)]
pub struct EffectivePhysiology {
    /// The stored profile, as the reader may see it; `None` when none is
    /// stored (no profile is invented in its place).
    pub profile: Option<UserPhysiologicalProfile>,
    /// The latest measurement of each anchor.
    pub measured: AnchorMeasurements,
}

impl EffectivePhysiology {
    /// The resting heart rate, bpm: the profile's, else the latest measured.
    #[must_use]
    pub fn resting_hr(&self) -> Option<u16> {
        self.profile
            .as_ref()
            .and_then(|p| p.resting_hr)
            .or_else(|| {
                self.measured
                    .resting_hr
                    .as_ref()
                    .and_then(|m| u16::try_from(m.value).ok())
            })
    }

    /// The `VO2max`, ml/kg/min: the profile's, else the latest measured.
    #[must_use]
    pub fn vo2_max(&self) -> Option<f64> {
        self.profile
            .as_ref()
            .and_then(|p| p.vo2_max)
            .or_else(|| self.measured.vo2_max.as_ref().map(|m| m.value))
    }

    /// The weight, kg: the profile's, else the latest measured.
    #[must_use]
    pub fn weight(&self) -> Option<f64> {
        self.profile
            .as_ref()
            .and_then(|p| p.weight)
            .or_else(|| self.measured.weight_kg.as_ref().map(|m| m.value))
    }

    /// The stored profile with each anchor it leaves empty filled from the
    /// latest measurement, for a reader that takes a whole profile; `None`
    /// when no profile is stored.
    #[must_use]
    pub fn filled_profile(&self) -> Option<UserPhysiologicalProfile> {
        let mut profile = self.profile.clone()?;
        profile.resting_hr = self.resting_hr();
        profile.vo2_max = self.vo2_max();
        profile.weight = self.weight();
        Some(profile)
    }
}

/// The athlete's effective physiology in `tenant`, for the anchors `wanted`
/// names.
///
/// A measurement is read only for a wanted anchor the profile leaves empty,
/// so a reader pays for, and is shown, no more of the wellness rows than it
/// uses. The profile is withheld from a reader its transport policy does not
/// admit (carnet#769), and the measurements are read under the provider terms
/// in force, as every wellness read is.
///
/// A measurement read that fails is logged and read as no measurement: it
/// costs the reader the device's number, never the profile it already holds.
///
/// # Errors
///
/// Returns the repository's error when the profile cannot be read.
pub async fn effective_physiology(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    tenant: TenantId,
    user_id: Uuid,
    wanted: Anchors,
) -> AppResult<EffectivePhysiology> {
    let profile = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .filter(|profile| ai_scope::admit_derived(profile.transport_policy));
    let held = profile.as_ref();
    let lacking = Anchors {
        resting_hr: wanted.resting_hr && held.and_then(|p| p.resting_hr).is_none(),
        vo2_max: wanted.vo2_max && held.and_then(|p| p.vo2_max).is_none(),
        weight: wanted.weight && held.and_then(|p| p.weight).is_none(),
    };
    let measured = if lacking == Anchors::default() {
        AnchorMeasurements::default()
    } else {
        stored_health::latest_anchor_measurements(repos, terms, user_id, &tenant, lacking)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    user_id = %user_id,
                    tenant_id = %tenant,
                    error = %e,
                    "latest anchor measurements unreadable; the stored profile stands alone"
                );
                AnchorMeasurements::default()
            })
    };
    Ok(EffectivePhysiology { profile, measured })
}

/// The measurements one statement of the athlete's carries; each `None`
/// when it does not set it. A `VO2max` is not among them (module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ManualMeasurements {
    /// Resting heart rate, bpm.
    pub resting_hr: Option<u16>,
    /// Weight, kg.
    pub weight_kg: Option<f64>,
}

/// What [`record_manual_measurements`] did with a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualFiling {
    /// Filed as the day's measurements under `athlete`.
    Filed,
    /// The statement carried no measurement; nothing was written.
    Nothing,
    /// Stated in a derivation that served first-party-only data, which no
    /// wellness row can carry: the anchor alone holds it.
    Withheld,
}

/// File the measurements the athlete stated as today's.
///
/// Under the `athlete` key, in their own zone's calendar day: the resting
/// heart rate on the day's recovery row, the weight on its body row. Each is
/// written as a partial summary, so stating one reading never clears another
/// the athlete gave the same day, and stating it again rewrites the same
/// row. Nothing is written when the statement carries neither, or when the
/// derivation running here served first-party-only data (module docs).
///
/// # Errors
///
/// Returns the store's error when the data source or a row cannot be written.
pub async fn record_manual_measurements(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    stated: ManualMeasurements,
) -> AppResult<ManualFiling> {
    if stated == ManualMeasurements::default() {
        return Ok(ManualFiling::Nothing);
    }
    if ai_scope::derived_policy().is_first_party_only() {
        info!(
            user_id = %user_id,
            tenant_id = %tenant,
            "stated measurements kept on the profile only: the turn served first-party-only data"
        );
        return Ok(ManualFiling::Withheld);
    }
    let user = user_id.to_string();
    let store = WellnessStore::new(repos);
    let source_id = store
        .record_data_source(
            &tenant,
            &DataSource {
                id: Uuid::new_v4().to_string(),
                user_id: user.clone(),
                provider: ATHLETE.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await?;
    let now = Utc::now();
    let date = now
        .with_timezone(&athlete_zone(repos, user_id).await)
        .date_naive();
    let mut writes = store.writes("manual entry");
    if let Some(resting_hr) = stated.resting_hr {
        let row = StoredRecoveryMetrics {
            id: String::new(),
            user_id: user.clone(),
            data_source_id: source_id.clone(),
            date,
            recovery_score: None,
            readiness_score: None,
            hrv_ms: None,
            hrv_rmssd: None,
            resting_heart_rate: Some(u32::from(resting_hr)),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: None,
            source_name: ATHLETE.to_owned(),
            recorded_at: now,
        };
        writes
            .recovery(tenant, &row, WellnessWriteMode::PartialSummary, now)
            .await?;
    }
    if let Some(weight_kg) = stated.weight_kg {
        let row = StoredHealthMetrics {
            id: String::new(),
            user_id: user,
            data_source_id: source_id,
            date,
            weight_kg: Some(weight_kg),
            body_fat_pct: None,
            muscle_mass_kg: None,
            bmi: None,
            bone_mass_kg: None,
            water_pct: None,
            systolic_bp: None,
            diastolic_bp: None,
            blood_glucose: None,
            vo2_max: None,
            source_name: ATHLETE.to_owned(),
            recorded_at: now,
        };
        writes
            .body(tenant, &row, WellnessWriteMode::PartialSummary, now)
            .await?;
    }
    Ok(ManualFiling::Filed)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use pierre_core::models::SportType;

    use super::*;
    use crate::stored_health::Measured;

    fn measured<T>(value: T) -> Measured<T> {
        Measured {
            value,
            date: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap_or_default(),
            source: "garmin".to_owned(),
        }
    }

    #[test]
    fn a_stored_anchor_stands_over_a_newer_measurement() {
        let mut profile = UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Run);
        profile.resting_hr = Some(48);
        let effective = EffectivePhysiology {
            profile: Some(profile),
            measured: AnchorMeasurements {
                resting_hr: Some(measured(55)),
                vo2_max: Some(measured(52.0)),
                weight_kg: None,
            },
        };
        assert_eq!(
            effective.resting_hr(),
            Some(48),
            "bug: the athlete's own anchor is replaced by a device's"
        );
        assert_eq!(
            effective.vo2_max(),
            Some(52.0),
            "an empty anchor takes the latest measurement"
        );
        assert_eq!(effective.weight(), None, "absent stays absent");
        let filled = effective
            .filled_profile()
            .unwrap_or_else(|| UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run));
        assert_eq!(filled.resting_hr, Some(48), "the filled profile keeps it");
        assert_eq!(filled.vo2_max, Some(52.0), "and fills the empty one");
    }

    #[test]
    fn no_profile_is_invented_from_measurements() {
        let effective = EffectivePhysiology {
            profile: None,
            measured: AnchorMeasurements {
                resting_hr: Some(measured(55)),
                vo2_max: None,
                weight_kg: Some(measured(71.8)),
            },
        };
        assert_eq!(effective.resting_hr(), Some(55), "the measured value");
        assert_eq!(effective.weight(), Some(71.8), "the measured weight");
        assert!(
            effective.filled_profile().is_none(),
            "bug: a profile with default sport and level was invented"
        );
    }
}
