// ABOUTME: Whether a coach-linked member's wellness syncs through their human coach's API key, decided at each sync
// ABOUTME: Only with the member's wellness consent, never beside a source of their own, never a field the athlete keeps private

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Delegated wellness
//!
//! A member whose human coach reads them on a coaching platform through a
//! confirmed link can let that link sync their wellness too — sleep, HRV,
//! resting heart rate, weight, the daily note — by granting their coach
//! `wellness_read` ([`crate::coach_consents`]). The scheduled sync
//! then reads it through the coach's own credential, at the member's athlete
//! path, and files it under the tenant the member confirmed in, where ending
//! the link or withdrawing the consent deletes it again.
//!
//! [`delegated_wellness_credentials`] decides every such sync and holds every
//! exclusion:
//!
//! - the delegated read decision ([`member_read`]) must serve the member
//!   through the coach: a credential of their own for the platform wins, the
//!   link must stand and bind by email, and the coach's session must be live;
//! - the member must have granted the link `wellness_read`, read when the
//!   sync runs, not when its pass listed the link;
//! - the member holds no wellness source of their own in the tenant the link
//!   files under ([`excluded_from_wellness`]): their own Garmin or WHOOP
//!   connection stays the source of record for what it measures;
//! - the coach's roster must list the link's athlete and say which wellness
//!   fields the athlete keeps private from the coach ([`PrivateWellness`]); a
//!   roster that leaves it unknown syncs nothing.
//!
//! The roster is read as every roster read on the coach's path reads it
//! ([`coach_roster`], under the link's coach tenant): the ten-minute cache
//! when it is warm, otherwise one live read with a live read's side effects —
//! a link whose athlete left the roster ends, and both sides are told. The
//! scheduled pass runs inside the background share of the request budget, so
//! that read is counted against the coach's key like the sync's own.
//!
//! A failure anywhere syncs nothing for that member and is logged with the
//! link id. A coach key the platform no longer honours is logged and nothing
//! more: flagging the coach's connection and telling them belongs to the tool
//! runtime, above this crate, and the coach meets it on their next read.
//!
//! What the link files is what the consent names, and nothing else the
//! platform's day carries: sleep as the night's duration and heart rate
//! ([`sleep_the_link_files`]), recovery as HRV, resting heart rate and the
//! daily note ([`recovery_the_link_files`]), health as the weight alone
//! ([`health_the_link_files`]). Blood oxygen, breathing rate, body fat, blood
//! pressure and `VO2max` are never stored through a link. Of what the consent
//! names, a field the athlete keeps private from the coach is dropped too, and
//! a row left with nothing to file is not filed at all.

use dravr_enforme::models::connection::{CredentialKind, ProviderCredentials};
use dravr_equilibre_sync::{StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession};
use pierre_core::errors::AppResult;
use pierre_core::models::{DelegatedConnection, RosterAthlete};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::sync_backend;
use secrecy::ExposeSecret;
use std::fmt;
use tracing::{info, warn};
use uuid::Uuid;

use crate::coach_consents::{link_consents, wellness_read_skipped, WellnessReadSkip};
use crate::coach_platform::{coach_platform, CoachPlatform};
use crate::delegated_connections::{coach_roster, DelegationServices};
use crate::delegated_read::{member_read, CoachRead, MemberRead, OwnLogins};
use crate::health_sync::request_gate;

/// What a coach-linked member's wellness syncs with through their human
/// coach's link: the coach's key, and what the athlete keeps private from it.
pub struct DelegatedWellness {
    /// The coach's API key, sent as one, addressing the link's athlete id and
    /// filed under the member, gated by the coach key's own request budget.
    pub credentials: ProviderCredentials,
    /// The fields the athlete keeps private from the coach, which the sync
    /// files none of.
    pub private: PrivateWellness,
}

impl fmt::Debug for DelegatedWellness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelegatedWellness")
            .field("credentials", &"[REDACTED]")
            .field("private", &self.private)
            .finish()
    }
}

/// The wellness fields an athlete keeps private from their human coach on
/// the platform, as the stored fields a link drops.
///
/// Read from the roster's `private_wellness_keys`, Intervals.icu's names of
/// the athlete's wellness fields ([`Self::from_keys`]). A link drops each
/// stored field such a name maps to, whether or not the platform withheld it
/// from the coach's key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivateWellness {
    /// HRV (`hrv`).
    hrv: bool,
    /// Resting heart rate (`restingHR`).
    resting_heart_rate: bool,
    /// The night itself (`sleepSecs`): a stored night is its duration, so
    /// none is filed without it.
    sleep: bool,
    /// Heart rate while asleep (`avgSleepingHR`).
    sleeping_heart_rate: bool,
    /// Body weight (`weight`).
    weight: bool,
    /// The athlete's daily note (`comments`).
    note: bool,
}

impl PrivateWellness {
    /// Every field a link files: what a sync whose private fields were never
    /// recorded keeps, so it files nothing.
    pub(crate) const ALL: Self = Self {
        hrv: true,
        resting_heart_rate: true,
        sleep: true,
        sleeping_heart_rate: true,
        weight: true,
        note: true,
    };

    /// The fields Intervals.icu's wellness field names `keys` map to, matched
    /// without regard to case.
    ///
    /// A name that maps to nothing a link files changes nothing: a reading
    /// Dravr does not read (`hrvSDNN`, `sleepScore`, `sleepQuality`,
    /// `menstrualPhase`, …) or one the consent leaves out (`spO2`,
    /// `respiration`, `bodyFat`, `systolic`, `diastolic`, `vo2max`).
    #[must_use]
    pub(crate) fn from_keys(keys: &[String]) -> Self {
        let mut private = Self::default();
        for key in keys {
            match key.to_ascii_lowercase().as_str() {
                "hrv" => private.hrv = true,
                "restinghr" => private.resting_heart_rate = true,
                "sleepsecs" => private.sleep = true,
                "avgsleepinghr" => private.sleeping_heart_rate = true,
                "weight" => private.weight = true,
                "comments" => private.note = true,
                _ => {}
            }
        }
        private
    }

    /// The fields either set keeps private.
    #[must_use]
    pub(crate) const fn union(self, other: Self) -> Self {
        Self {
            hrv: self.hrv || other.hrv,
            resting_heart_rate: self.resting_heart_rate || other.resting_heart_rate,
            sleep: self.sleep || other.sleep,
            sleeping_heart_rate: self.sleeping_heart_rate || other.sleeping_heart_rate,
            weight: self.weight || other.weight,
            note: self.note || other.note,
        }
    }
}

/// What `member`'s `provider` wellness syncs with through their human
/// coach's confirmed link, or `None` when it does not sync that way.
pub async fn delegated_wellness_credentials(
    services: DelegationServices<'_>,
    member: Uuid,
    provider: &str,
) -> Option<DelegatedWellness> {
    let backend = sync_backend(provider);
    let platform = coach_platform(services.registry, &backend)
        .filter(|platform| platform.reads_member_wellness())?;
    let read = read_through_coach(services, platform.as_ref(), member).await?;
    if !syncs_wellness(services.repos, &read.link).await {
        return None;
    }
    let athlete = roster_athlete(services, platform.as_ref(), &read).await?;
    // A roster that does not say what the athlete keeps private could be
    // hiding any field: nothing syncs until it says.
    let Some(keys) = athlete.private_wellness_keys.as_deref() else {
        info!(
            link_id = %read.link.id,
            "Delegated wellness not synced: the roster does not say which wellness fields the athlete keeps private"
        );
        return None;
    };
    let private = PrivateWellness::from_keys(keys);
    coach_key_credentials(read, member, backend).map(|credentials| DelegatedWellness {
        credentials,
        private,
    })
}

/// `member`'s read through their human coach's link, or `None` (logged)
/// when the delegated read decision serves them none that way.
async fn read_through_coach(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    member: Uuid,
) -> Option<CoachRead> {
    match member_read(services.repos, services.registry, platform, member, None).await {
        Ok(MemberRead::ThroughCoach(read)) => Some(*read),
        Ok(MemberRead::Refused { link, refusal }) => {
            info!(
                link_id = %link.id,
                ?refusal,
                "Delegated wellness not synced: the link serves no read"
            );
            None
        }
        Ok(MemberRead::OwnCredential | MemberRead::NotLinked) => None,
        Err(e) => {
            warn!(
                user_id = %member,
                error = %e,
                "Delegated wellness not synced: the member's link could not be read"
            );
            None
        }
    }
}

/// Whether nothing in Dravr's own rows keeps `link`'s wellness from syncing
/// ([`excluded_from_wellness`]); what does is logged.
async fn syncs_wellness(repos: &RepositoryRegistry, link: &DelegatedConnection) -> bool {
    match excluded_from_wellness(repos, link).await {
        Ok(None) => true,
        Ok(Some(why)) => {
            info!(link_id = %link.id, why, "Delegated wellness not synced");
            false
        }
        Err(e) => {
            warn!(
                link_id = %link.id,
                error = %e,
                "Delegated wellness not synced: its exclusions could not be read"
            );
            false
        }
    }
}

/// The coach's roster entry for `read`'s athlete, read under the link's
/// coach tenant, or `None` (logged) when the roster cannot be read or no
/// longer lists the athlete.
async fn roster_athlete(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    read: &CoachRead,
) -> Option<RosterAthlete> {
    let link = &read.link;
    // A dead coach key fails this read with the reconnect refusal; it is
    // logged here and nothing more (see the module doc).
    let roster = match coach_roster(
        services,
        platform,
        link.coach_user_id,
        read.coach_tenant,
        false,
    )
    .await
    {
        Ok(roster) => roster,
        Err(e) => {
            warn!(
                link_id = %link.id,
                error = %e,
                "Delegated wellness not synced: the coach's roster could not be read"
            );
            return None;
        }
    };
    let athlete = roster
        .into_iter()
        .find(|athlete| athlete.id == link.provider_athlete_id);
    if athlete.is_none() {
        info!(
            link_id = %link.id,
            "Delegated wellness not synced: the coach's roster no longer lists the athlete"
        );
    }
    athlete
}

/// The coach's key `read` carries, as the credentials `member`'s `backend`
/// wellness syncs with: sent as an API key, at the link's athlete path,
/// counted against the key's own budget.
fn coach_key_credentials(
    read: CoachRead,
    member: Uuid,
    backend: String,
) -> Option<ProviderCredentials> {
    let CoachRead {
        link, credentials, ..
    } = read;
    let Some(access_token) = credentials.access_token else {
        warn!(
            link_id = %link.id,
            "Delegated wellness not synced: the coach's credential carries no key"
        );
        return None;
    };
    Some(ProviderCredentials {
        access_token: access_token.expose_secret().to_owned(),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
        user_id: member.to_string(),
        provider: backend,
        provider_user_id: Some(link.provider_athlete_id),
        kind: CredentialKind::ApiKey,
        request_gate: credentials.request_budget.map(request_gate),
    })
}

/// Why `link`'s member's wellness does not sync through it, read from
/// Dravr's own rows alone, without reading the platform; `None` when nothing
/// here keeps it from syncing.
///
/// - `no_wellness_consent`: the member has not granted their coach
///   `wellness_read` ([`link_consents`]);
/// - `unconfirmed`: the link names no tenant to file under, which only a
///   link never confirmed lacks;
/// - `own_connection` or `own_wellness_source`: a source of the member's own
///   wins over the link ([`wellness_read_skipped`]), the same reason both
///   apps show beside the consent.
///
/// # Errors
///
/// Returns the repository error when the member's credentials or connections
/// cannot be read.
pub(crate) async fn excluded_from_wellness(
    repos: &RepositoryRegistry,
    link: &DelegatedConnection,
) -> AppResult<Option<&'static str>> {
    if !link_consents(repos.groups.as_ref(), link)
        .await?
        .wellness_read
    {
        return Ok(Some("no_wellness_consent"));
    }
    if link.member_tenant_id.is_none() {
        return Ok(Some("unconfirmed"));
    }
    Ok(wellness_read_skipped(OwnLogins::of(repos), link)
        .await?
        .map(WellnessReadSkip::as_str))
}

/// `value` unless the athlete keeps it `private`.
fn unless<T>(private: bool, value: Option<T>) -> Option<T> {
    value.filter(|_| !private)
}

/// The part of a sleep row a link files under the member's `wellness_read`
/// consent: the night's span and duration, its heart rate and the HRV
/// measured in it, less what the athlete keeps `private`. Every other reading
/// a night can carry (its stages, efficiency, a score of any kind) is
/// dropped, so a reading the athlete keeps private under a name no field
/// here maps (`sleepScore`, `sleepQuality`) is never filed whatever the
/// platform's row later carries. A night whose duration is private is not
/// filed at all: a stored night is its duration.
pub(crate) fn sleep_the_link_files(
    session: &StoredSleepSession,
    private: PrivateWellness,
) -> Option<StoredSleepSession> {
    if private.sleep {
        return None;
    }
    Some(StoredSleepSession {
        id: session.id.clone(),
        user_id: session.user_id.clone(),
        data_source_id: session.data_source_id.clone(),
        is_nap: session.is_nap,
        start_datetime: session.start_datetime,
        end_datetime: session.end_datetime,
        total_sleep_seconds: session.total_sleep_seconds,
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: None,
        sleep_efficiency: None,
        avg_heart_rate: unless(private.sleeping_heart_rate, session.avg_heart_rate),
        min_heart_rate: unless(private.sleeping_heart_rate, session.min_heart_rate),
        avg_hrv: unless(private.hrv, session.avg_hrv),
        sleep_score: None,
        avg_spo2: None,
        stages: Vec::new(),
        source_name: session.source_name.clone(),
    })
}

/// The part of a recovery row a link files under the member's
/// `wellness_read` consent: HRV, resting heart rate and the daily note, less
/// what the athlete keeps `private`. Every other reading the platform's row
/// carries (blood oxygen, breathing rate, a score of any kind) is dropped,
/// and a row left with none of the three is not filed at all.
pub(crate) fn recovery_the_link_files(
    metrics: &StoredRecoveryMetrics,
    private: PrivateWellness,
) -> Option<StoredRecoveryMetrics> {
    let hrv_ms = unless(private.hrv, metrics.hrv_ms);
    let hrv_rmssd = unless(private.hrv, metrics.hrv_rmssd);
    let resting_heart_rate = unless(private.resting_heart_rate, metrics.resting_heart_rate);
    let athlete_note = unless(private.note, metrics.athlete_note.clone());
    let named = hrv_ms.is_some()
        || hrv_rmssd.is_some()
        || resting_heart_rate.is_some()
        || athlete_note.is_some();
    named.then(|| StoredRecoveryMetrics {
        id: metrics.id.clone(),
        user_id: metrics.user_id.clone(),
        data_source_id: metrics.data_source_id.clone(),
        date: metrics.date,
        recovery_score: None,
        readiness_score: None,
        hrv_ms,
        hrv_rmssd,
        resting_heart_rate,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note,
        source_name: metrics.source_name.clone(),
        recorded_at: metrics.recorded_at,
    })
}

/// The part of a health row a link files under the member's `wellness_read`
/// consent: the weight alone, unless the athlete keeps it `private`. Body
/// fat, blood pressure, `VO2max` and every other body reading are dropped,
/// and a row with no weight is not filed.
pub(crate) fn health_the_link_files(
    snapshot: &StoredHealthMetrics,
    private: PrivateWellness,
) -> Option<StoredHealthMetrics> {
    unless(private.weight, snapshot.weight_kg).map(|weight| StoredHealthMetrics {
        id: snapshot.id.clone(),
        user_id: snapshot.user_id.clone(),
        data_source_id: snapshot.data_source_id.clone(),
        date: snapshot.date,
        weight_kg: Some(weight),
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: None,
        vo2_max: None,
        source_name: snapshot.source_name.clone(),
        recorded_at: snapshot.recorded_at,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};
    use pierre_core::constants::oauth_providers::INTERVALS_ICU;

    use super::*;

    fn recovery(
        hrv: Option<f64>,
        resting_hr: Option<u32>,
        note: Option<&str>,
    ) -> StoredRecoveryMetrics {
        StoredRecoveryMetrics {
            id: "intervals_icu-recovery-m1-2026-10-14".to_owned(),
            user_id: "m1".to_owned(),
            data_source_id: "intervals_icu-default".to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
            recovery_score: Some(80),
            readiness_score: Some(70),
            hrv_ms: hrv,
            hrv_rmssd: hrv,
            resting_heart_rate: resting_hr,
            stress_score: Some(20),
            body_battery: Some(60),
            spo2: Some(97.0),
            respiratory_rate: Some(14.5),
            skin_temp_deviation: Some(0.2),
            daily_strain: Some(9.0),
            athlete_note: note.map(str::to_owned),
            source_name: INTERVALS_ICU.to_owned(),
            recorded_at: Utc::now(),
        }
    }

    fn health(weight: Option<f64>) -> StoredHealthMetrics {
        StoredHealthMetrics {
            id: "intervals_icu-health-m1-2026-10-14".to_owned(),
            user_id: "m1".to_owned(),
            data_source_id: "intervals_icu-default".to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
            weight_kg: weight,
            body_fat_pct: Some(12.0),
            muscle_mass_kg: Some(33.0),
            bmi: Some(22.0),
            bone_mass_kg: Some(3.1),
            water_pct: Some(58.0),
            systolic_bp: Some(118),
            diastolic_bp: Some(76),
            blood_glucose: Some(5.1),
            vo2_max: Some(55.0),
            source_name: INTERVALS_ICU.to_owned(),
            recorded_at: Utc::now(),
        }
    }

    fn sleep() -> StoredSleepSession {
        let start = Utc::now();
        StoredSleepSession {
            id: "intervals_icu-sleep-m1-2026-10-14".to_owned(),
            user_id: "m1".to_owned(),
            data_source_id: "intervals_icu-default".to_owned(),
            is_nap: false,
            start_datetime: start,
            end_datetime: start + chrono::Duration::hours(7),
            total_sleep_seconds: Some(25_200),
            deep_sleep_seconds: None,
            light_sleep_seconds: None,
            rem_sleep_seconds: None,
            awake_seconds: None,
            sleep_efficiency: None,
            avg_heart_rate: Some(52.0),
            min_heart_rate: Some(44),
            avg_hrv: Some(70.0),
            sleep_score: None,
            avg_spo2: None,
            stages: Vec::new(),
            source_name: INTERVALS_ICU.to_owned(),
        }
    }

    fn keys(names: &[&str]) -> PrivateWellness {
        let names: Vec<String> = names.iter().map(|&name| name.to_owned()).collect();
        PrivateWellness::from_keys(&names)
    }

    const NOTHING: PrivateWellness = PrivateWellness {
        hrv: false,
        resting_heart_rate: false,
        sleep: false,
        sleeping_heart_rate: false,
        weight: false,
        note: false,
    };

    #[test]
    fn a_link_files_the_recovery_the_consent_names_and_nothing_else() {
        let kept =
            recovery_the_link_files(&recovery(Some(62.0), Some(47), Some("legs heavy")), NOTHING)
                .expect("HRV, resting heart rate and the note are named");
        assert_eq!(kept.hrv_rmssd, Some(62.0));
        assert_eq!(kept.hrv_ms, Some(62.0));
        assert_eq!(kept.resting_heart_rate, Some(47));
        assert_eq!(kept.athlete_note.as_deref(), Some("legs heavy"));
        assert_eq!(kept.spo2, None);
        assert_eq!(kept.respiratory_rate, None);
        assert_eq!(kept.skin_temp_deviation, None);
        assert_eq!(
            (kept.recovery_score, kept.readiness_score, kept.stress_score),
            (None, None, None)
        );
        assert_eq!((kept.body_battery, kept.daily_strain), (None, None));

        // A note alone is a named reading; blood oxygen and breathing alone
        // are not.
        assert!(
            recovery_the_link_files(&recovery(None, None, Some("rest day")), NOTHING).is_some()
        );
        assert!(recovery_the_link_files(&recovery(None, None, None), NOTHING).is_none());
    }

    #[test]
    fn a_link_files_the_weight_alone_and_no_row_without_one() {
        let kept =
            health_the_link_files(&health(Some(71.5)), NOTHING).expect("the weight is named");
        assert_eq!(kept.weight_kg, Some(71.5));
        assert_eq!((kept.systolic_bp, kept.diastolic_bp), (None, None));
        assert_eq!((kept.body_fat_pct, kept.vo2_max), (None, None));
        assert_eq!((kept.muscle_mass_kg, kept.bmi), (None, None));
        assert_eq!((kept.bone_mass_kg, kept.water_pct), (None, None));
        assert_eq!(kept.blood_glucose, None);
        assert!(health_the_link_files(&health(None), NOTHING).is_none());
    }

    /// Each Intervals.icu name drops the stored field it maps to, in any
    /// case; a name for a reading no link files changes nothing.
    #[test]
    fn each_private_key_drops_the_field_it_names() {
        assert_eq!(keys(&[]), NOTHING);
        assert_eq!(
            keys(&[
                "menstrualPhase",
                "hrvSDNN",
                "sleepScore",
                "sleepQuality",
                "spO2",
                "bodyFat"
            ]),
            NOTHING
        );

        let full = recovery(Some(62.0), Some(47), Some("legs heavy"));
        let no_hrv = recovery_the_link_files(&full, keys(&["hrv"])).expect("a row stays");
        assert_eq!((no_hrv.hrv_ms, no_hrv.hrv_rmssd), (None, None));
        assert_eq!(no_hrv.resting_heart_rate, Some(47));
        assert_eq!(no_hrv.athlete_note.as_deref(), Some("legs heavy"));
        let no_rhr = recovery_the_link_files(&full, keys(&["RESTINGHR"])).expect("a row stays");
        assert_eq!(
            (no_rhr.hrv_rmssd, no_rhr.resting_heart_rate),
            (Some(62.0), None)
        );
        let no_note = recovery_the_link_files(&full, keys(&["comments"])).expect("a row stays");
        assert_eq!(no_note.athlete_note, None);
        assert!(
            recovery_the_link_files(&full, keys(&["hrv", "restingHR", "comments"])).is_none(),
            "a row left empty is not filed"
        );

        assert!(health_the_link_files(&health(Some(71.5)), keys(&["weight"])).is_none());

        let night = sleep_the_link_files(&sleep(), keys(&["avgSleepingHR", "hrv"]))
            .expect("the night stays");
        assert_eq!(night.total_sleep_seconds, Some(25_200));
        assert_eq!((night.avg_heart_rate, night.min_heart_rate), (None, None));
        assert_eq!(night.avg_hrv, None);
        let night = sleep_the_link_files(&sleep(), NOTHING).expect("the night as it comes");
        assert_eq!(night.avg_heart_rate, Some(52.0));
        assert!(
            sleep_the_link_files(&sleep(), keys(&["sleepSecs"])).is_none(),
            "a night is its duration"
        );
    }

    /// A night files its span, duration, heart rate and HRV and nothing else
    /// it carries: a stage breakdown, an efficiency or a score could be what
    /// the athlete keeps private under a name no field maps (`sleepScore`,
    /// `sleepQuality`), so none of them is filed.
    #[test]
    fn a_link_files_the_night_and_nothing_else() {
        let mut night = sleep();
        night.deep_sleep_seconds = Some(5_400);
        night.light_sleep_seconds = Some(14_400);
        night.rem_sleep_seconds = Some(5_400);
        night.awake_seconds = Some(1_200);
        night.sleep_efficiency = Some(92.0);
        night.sleep_score = Some(81);
        night.stages = vec![dravr_equilibre_sync::SleepStage {
            stage_type: dravr_equilibre_sync::SleepStageType::Deep,
            start: night.start_datetime,
            end: night.start_datetime + chrono::Duration::minutes(90),
            duration_seconds: 5_400,
        }];

        let kept = sleep_the_link_files(&night, keys(&["sleepScore", "sleepQuality"]))
            .expect("the night stays");
        assert_eq!(
            (kept.start_datetime, kept.end_datetime),
            (night.start_datetime, night.end_datetime)
        );
        assert_eq!(kept.total_sleep_seconds, Some(25_200));
        assert_eq!(
            (kept.avg_heart_rate, kept.min_heart_rate),
            (Some(52.0), Some(44))
        );
        assert_eq!(kept.avg_hrv, Some(70.0));
        assert_eq!(
            (kept.deep_sleep_seconds, kept.light_sleep_seconds),
            (None, None)
        );
        assert_eq!((kept.rem_sleep_seconds, kept.awake_seconds), (None, None));
        assert_eq!((kept.sleep_efficiency, kept.sleep_score), (None, None));
        assert!(kept.stages.is_empty());
    }

    #[test]
    fn a_sync_that_recorded_nothing_files_nothing() {
        assert!(sleep_the_link_files(&sleep(), PrivateWellness::ALL).is_none());
        assert!(recovery_the_link_files(
            &recovery(Some(62.0), Some(47), Some("legs heavy")),
            PrivateWellness::ALL
        )
        .is_none());
        assert!(health_the_link_files(&health(Some(71.5)), PrivateWellness::ALL).is_none());
        assert_eq!(
            keys(&["hrv"]).union(keys(&["weight"])),
            keys(&["weight", "hrv"])
        );
    }
}
