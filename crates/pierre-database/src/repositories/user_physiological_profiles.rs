// ABOUTME: UserPhysiologicalProfileRepository + DossierRepository traits plus the one shared implementation both backends emit
// ABOUTME: Backs the Endurance latest-snapshot and dossier exports; the two zone-set JSON columns are opaque blobs bound as serde_json::Value
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::zones::{HrZoneSet, PowerZoneSet};
use pierre_core::models::{
    Dossier, MeasurementKind, MetricProvenance, ProvenancedValue, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_core::transport::TransportPolicy;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

/// Typed CRUD for [`UserPhysiologicalProfile`] backed by the
/// `user_physiological_profiles` table.
///
/// Row layout: see `migrations/20260430000003_user_profile_endurance_fields.sql`
/// (`SQLite`) and `migrations_pg/20260430000003_user_profile_endurance_fields.sql`
/// (`PostgreSQL`).
///
/// Every method scopes by `tenant_id` to satisfy the multi-tenant isolation
/// invariant in CLAUDE.md.
#[async_trait]
pub trait UserPhysiologicalProfileRepository: Send + Sync {
    /// Insert or update the profile row for `(tenant_id, user_id)`.
    ///
    /// `profile.user_id` must match `user_id`; the implementation rejects
    /// mismatches with [`pierre_core::errors::AppError`] to prevent
    /// cross-user writes from a confused caller.
    async fn upsert_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        profile: &UserPhysiologicalProfile,
    ) -> AppResult<()>;

    /// Fetch the profile for `(tenant_id, user_id)`. Returns `None` when
    /// the user has no row yet.
    async fn get_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<UserPhysiologicalProfile>>;
}

/// Read-time composer for the Endurance [`Dossier`] aggregate.
///
/// Per the locked architectural decision the dossier is **not** persisted as
/// its own row — the implementation pulls from the existing tables
/// (`user_physiological_profiles` for physiology + zones, `user_profiles`
/// JSON column for goals / nutrition / equipment) and assembles the
/// aggregate per request. Cache invalidation is therefore unnecessary on
/// the dossier itself; only the underlying tables need cache hooks.
#[async_trait]
pub trait DossierRepository: Send + Sync {
    /// Compose the dossier for `(tenant_id, user_id)`.
    ///
    /// Returns an empty dossier shell (all slots `None` / empty) when the
    /// user has no underlying rows so the API endpoint can return a 200
    /// rather than a 404 for fresh accounts.
    ///
    /// `readable` is the strictest stamp a fact may carry and still be read
    /// (the caller passes `ai_scope::readable_policy()`), bound into the fact
    /// reads so their limits count only readable rows.
    ///
    /// `admit` decides, per stored row, whether content stamped with a policy
    /// may be served to this caller (carnet#769): the caller passes
    /// `ai_scope::admit_derived`, so a first-party-only fact or profile never
    /// reaches an external one. A row it refuses is left out of the dossier.
    async fn compose_dossier(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        readable: TransportPolicy,
        admit: &(dyn Fn(TransportPolicy) -> bool + Sync),
    ) -> AppResult<Dossier>;
}

/// Write or refresh the one profile row per `(tenant_id, user_id)`.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres. `user_id` binds as [`pierre_core::models::UserId`] and
/// `tenant_id` as [`TenantId`], each of which encodes as hyphenated text on
/// `SQLite` and as a native `uuid` on Postgres. The `INTEGER` columns bind
/// as `i32` (Postgres `INTEGER` is four bytes; `SQLite` takes any width).
/// The two zone-set columns bind as [`Value`], which sqlx stores as `TEXT`
/// on `SQLite` and `jsonb` on Postgres: the stored JSON is an opaque blob
/// whose only reader is [`profile_from_row`], which parses it back into
/// [`HrZoneSet`] / [`PowerZoneSet`]. `CURRENT_TIMESTAMP` is the spelling
/// both engines accept.
pub(crate) const UPSERT_PHYSIOLOGICAL_PROFILE_SQL: &str = r"
            INSERT INTO user_physiological_profiles (
                user_id, tenant_id, vo2_max, resting_hr, max_hr,
                lactate_threshold_percentage, age, weight, fitness_level,
                primary_sport, training_experience_years, ftp_watts,
                threshold_pace_sec_per_km, hr_zones_json, power_zones_json,
                threshold_hr,
                critical_power_watts, critical_power_watts_kind, critical_power_watts_origin, critical_power_watts_as_of,
                w_prime_joules, w_prime_joules_kind, w_prime_joules_origin, w_prime_joules_as_of,
                critical_speed_mps, critical_speed_mps_kind, critical_speed_mps_origin, critical_speed_mps_as_of,
                d_prime_meters, d_prime_meters_kind, d_prime_meters_origin, d_prime_meters_as_of,
                created_at, updated_at, first_party_only
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                    $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28,
                    $29, $30, $31, $32, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, $33)
            ON CONFLICT (tenant_id, user_id) DO UPDATE SET
                vo2_max = EXCLUDED.vo2_max,
                resting_hr = EXCLUDED.resting_hr,
                max_hr = EXCLUDED.max_hr,
                lactate_threshold_percentage = EXCLUDED.lactate_threshold_percentage,
                age = EXCLUDED.age,
                weight = EXCLUDED.weight,
                fitness_level = EXCLUDED.fitness_level,
                primary_sport = EXCLUDED.primary_sport,
                training_experience_years = EXCLUDED.training_experience_years,
                ftp_watts = EXCLUDED.ftp_watts,
                threshold_pace_sec_per_km = EXCLUDED.threshold_pace_sec_per_km,
                hr_zones_json = EXCLUDED.hr_zones_json,
                power_zones_json = EXCLUDED.power_zones_json,
                threshold_hr = EXCLUDED.threshold_hr,
                critical_power_watts = EXCLUDED.critical_power_watts,
                critical_power_watts_kind = EXCLUDED.critical_power_watts_kind,
                critical_power_watts_origin = EXCLUDED.critical_power_watts_origin,
                critical_power_watts_as_of = EXCLUDED.critical_power_watts_as_of,
                w_prime_joules = EXCLUDED.w_prime_joules,
                w_prime_joules_kind = EXCLUDED.w_prime_joules_kind,
                w_prime_joules_origin = EXCLUDED.w_prime_joules_origin,
                w_prime_joules_as_of = EXCLUDED.w_prime_joules_as_of,
                critical_speed_mps = EXCLUDED.critical_speed_mps,
                critical_speed_mps_kind = EXCLUDED.critical_speed_mps_kind,
                critical_speed_mps_origin = EXCLUDED.critical_speed_mps_origin,
                critical_speed_mps_as_of = EXCLUDED.critical_speed_mps_as_of,
                d_prime_meters = EXCLUDED.d_prime_meters,
                d_prime_meters_kind = EXCLUDED.d_prime_meters_kind,
                d_prime_meters_origin = EXCLUDED.d_prime_meters_origin,
                d_prime_meters_as_of = EXCLUDED.d_prime_meters_as_of,
                updated_at = CURRENT_TIMESTAMP,
                first_party_only = EXCLUDED.first_party_only
            ";

/// Read the one profile row for `(tenant_id, user_id)`.
pub(crate) const GET_PHYSIOLOGICAL_PROFILE_SQL: &str = r"
            SELECT vo2_max, resting_hr, max_hr, lactate_threshold_percentage,
                   age, weight, fitness_level, primary_sport,
                   training_experience_years, ftp_watts,
                   threshold_pace_sec_per_km, hr_zones_json, power_zones_json,
                   threshold_hr,
                   critical_power_watts, critical_power_watts_kind, critical_power_watts_origin, critical_power_watts_as_of,
                   w_prime_joules, w_prime_joules_kind, w_prime_joules_origin, w_prime_joules_as_of,
                   critical_speed_mps, critical_speed_mps_kind, critical_speed_mps_origin, critical_speed_mps_as_of,
                   d_prime_meters, d_prime_meters_kind, d_prime_meters_origin, d_prime_meters_as_of,
                   first_party_only
            FROM user_physiological_profiles
            WHERE tenant_id = $1 AND user_id = $2
            LIMIT 1
            ";

/// The column values the upsert binds, converted from the typed profile.
///
/// `fitness_level` and `primary_sport` are stored as their serde JSON text
/// (a quoted string), which is what [`profile_from_row`] parses back.
pub(crate) struct ProfileBinds {
    pub(crate) fitness_level: String,
    pub(crate) primary_sport: String,
    pub(crate) ftp_watts: Option<i32>,
    pub(crate) hr_zones: Option<Value>,
    pub(crate) power_zones: Option<Value>,
    pub(crate) critical_power_watts: ProvenanceBinds<i32>,
    pub(crate) w_prime_joules: ProvenanceBinds<i32>,
    pub(crate) critical_speed_mps: ProvenanceBinds<f64>,
    pub(crate) d_prime_meters: ProvenanceBinds<f64>,
}

/// The four columns one [`ProvenancedValue`] is stored in: the value, its
/// kind, its origin and its as-of date. All four are `None` together when the
/// profile has no such value.
pub(crate) struct ProvenanceBinds<V> {
    pub(crate) value: Option<V>,
    pub(crate) kind: Option<&'static str>,
    pub(crate) origin: Option<String>,
    pub(crate) as_of: Option<NaiveDate>,
}

/// Split a provenanced value into its four column binds, converting the value
/// to its column type.
fn provenance_binds<T, V>(
    value: Option<&ProvenancedValue<T>>,
    convert: impl Fn(&T) -> AppResult<V>,
) -> AppResult<ProvenanceBinds<V>> {
    let Some(v) = value else {
        return Ok(ProvenanceBinds {
            value: None,
            kind: None,
            origin: None,
            as_of: None,
        });
    };
    Ok(ProvenanceBinds {
        value: Some(convert(&v.value)?),
        kind: Some(v.provenance.kind.as_str()),
        origin: v.provenance.origin.clone(),
        as_of: v.provenance.as_of,
    })
}

/// A whole-number value bound to an `INTEGER` column.
fn integer_bind(field: &'static str) -> impl Fn(&u32) -> AppResult<i32> {
    move |v| {
        i32::try_from(*v)
            .map_err(|_| AppError::invalid_input(format!("{field} is out of range: {v}")))
    }
}

/// Serialize the fields whose column type is not the field's own type.
///
/// # Errors
/// Returns a database error when a zone set or enum cannot be serialized,
/// and an invalid-input error when `ftp_watts` does not fit the `INTEGER`
/// column.
pub(crate) fn profile_binds(profile: &UserPhysiologicalProfile) -> AppResult<ProfileBinds> {
    let hr_zones = profile
        .hr_zones
        .as_ref()
        .map(|z| {
            serde_json::to_value(z)
                .map_err(|e| AppError::database(format!("serialize hr_zones: {e}")))
        })
        .transpose()?;
    let power_zones = profile
        .power_zones
        .as_ref()
        .map(|z| {
            serde_json::to_value(z)
                .map_err(|e| AppError::database(format!("serialize power_zones: {e}")))
        })
        .transpose()?;
    let fitness_level = serde_json::to_string(&profile.fitness_level)
        .map_err(|e| AppError::database(format!("serialize fitness_level: {e}")))?;
    let primary_sport = serde_json::to_string(&profile.primary_sport)
        .map_err(|e| AppError::database(format!("serialize primary_sport: {e}")))?;
    let ftp_watts = profile
        .ftp_watts
        .map(|v| {
            i32::try_from(v)
                .map_err(|_| AppError::invalid_input(format!("ftp_watts is out of range: {v}")))
        })
        .transpose()?;
    Ok(ProfileBinds {
        fitness_level,
        primary_sport,
        ftp_watts,
        hr_zones,
        power_zones,
        critical_power_watts: provenance_binds(
            profile.critical_power_watts.as_ref(),
            integer_bind("critical_power_watts"),
        )?,
        w_prime_joules: provenance_binds(
            profile.w_prime_joules.as_ref(),
            integer_bind("w_prime_joules"),
        )?,
        critical_speed_mps: provenance_binds(profile.critical_speed_mps.as_ref(), |v| Ok(*v))?,
        d_prime_meters: provenance_binds(profile.d_prime_meters.as_ref(), |v| Ok(*v))?,
    })
}

/// Rebuild the typed profile from one row of [`GET_PHYSIOLOGICAL_PROFILE_SQL`].
///
/// Every nullable numeric is decoded as `Option<T>`, never as a bare `T`
/// recovered with `.ok()`. `SQLite`'s C API answers a NULL column with 0
/// rather than an error, so the bare form turns "not measured" into a
/// measured zero — a weight of 0 kg reaching the TSS engine reads as real
/// data. Only a partially-filled profile shows it, which is why it survived
/// until `set_physiology` began writing one.
///
/// # Errors
/// Returns a database error when a column cannot be decoded or a stored
/// enum / zone set no longer parses.
pub(crate) fn profile_from_row<R>(row: &R, user_id: Uuid) -> AppResult<UserPhysiologicalProfile>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Value: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let fitness_level_str: String = row
        .try_get("fitness_level")
        .map_err(|e| AppError::database(format!("read fitness_level: {e}")))?;
    let fitness_level: FitnessLevel = serde_json::from_str(&fitness_level_str)
        .map_err(|e| AppError::database(format!("parse fitness_level: {e}")))?;
    let primary_sport_str: String = row
        .try_get("primary_sport")
        .map_err(|e| AppError::database(format!("read primary_sport: {e}")))?;
    let primary_sport: SportType = serde_json::from_str(&primary_sport_str)
        .map_err(|e| AppError::database(format!("parse primary_sport: {e}")))?;

    let resting_hr = row.try_get::<Option<i32>, _>("resting_hr").ok().flatten();
    let max_hr = row.try_get::<Option<i32>, _>("max_hr").ok().flatten();
    let age = row.try_get::<Option<i32>, _>("age").ok().flatten();
    let training_years = row
        .try_get::<Option<i32>, _>("training_experience_years")
        .ok()
        .flatten();
    let ftp_watts_db = row.try_get::<Option<i32>, _>("ftp_watts").ok().flatten();
    let threshold_hr = row.try_get::<Option<i32>, _>("threshold_hr").ok().flatten();

    let hr_zones_json: Option<Value> = row
        .try_get("hr_zones_json")
        .map_err(|e| AppError::database(format!("read hr_zones_json: {e}")))?;
    let power_zones_json: Option<Value> = row
        .try_get("power_zones_json")
        .map_err(|e| AppError::database(format!("read power_zones_json: {e}")))?;
    let hr_zones: Option<HrZoneSet> = hr_zones_json
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| AppError::database(format!("parse hr_zones_json: {e}")))?;
    let power_zones: Option<PowerZoneSet> = power_zones_json
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| AppError::database(format!("parse power_zones_json: {e}")))?;

    Ok(UserPhysiologicalProfile {
        user_id,
        vo2_max: row.try_get::<Option<f64>, _>("vo2_max").ok().flatten(),
        resting_hr: resting_hr.and_then(|v| u16::try_from(v).ok()),
        max_hr: max_hr.and_then(|v| u16::try_from(v).ok()),
        lactate_threshold_percentage: row
            .try_get::<Option<f64>, _>("lactate_threshold_percentage")
            .ok()
            .flatten(),
        threshold_hr: threshold_hr.and_then(|v| u16::try_from(v).ok()),
        age: age.and_then(|v| u16::try_from(v).ok()),
        weight: row.try_get::<Option<f64>, _>("weight").ok().flatten(),
        fitness_level,
        primary_sport,
        training_experience_years: training_years.and_then(|v| u8::try_from(v).ok()),
        ftp_watts: ftp_watts_db.and_then(|v| u32::try_from(v).ok()),
        threshold_pace_sec_per_km: row
            .try_get::<Option<f64>, _>("threshold_pace_sec_per_km")
            .ok()
            .flatten(),
        hr_zones,
        power_zones,
        critical_power_watts: provenanced_from_row(row, "critical_power_watts", |v: i32| {
            u32::try_from(v).ok()
        })?,
        w_prime_joules: provenanced_from_row(row, "w_prime_joules", |v: i32| {
            u32::try_from(v).ok()
        })?,
        critical_speed_mps: provenanced_from_row(row, "critical_speed_mps", Some::<f64>)?,
        d_prime_meters: provenanced_from_row(row, "d_prime_meters", Some::<f64>)?,
        transport_policy: TransportPolicy::from_first_party_only(
            row.try_get("first_party_only")
                .map_err(|e| AppError::database(format!("read first_party_only: {e}")))?,
        ),
    })
}

/// Read one provenanced value from the four columns
/// [`ProvenanceBinds`] wrote: `<column>`, `<column>_kind`, `<column>_origin`
/// and `<column>_as_of`.
///
/// A value with no kind, or a kind with no value, is refused rather than
/// guessed: a value whose kind is unknown would otherwise reach the athlete
/// with nothing to say whether it was measured.
///
/// # Errors
/// Returns a database error when a column cannot be decoded, the value does
/// not fit its type, or the value and its kind are not stored together.
fn provenanced_from_row<R, V, T>(
    row: &R,
    column: &str,
    convert: impl Fn(V) -> Option<T>,
) -> AppResult<Option<ProvenancedValue<T>>>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    V: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let read = |e: sqlx::Error, which: &str| AppError::database(format!("read {which}: {e}"));
    let kind_column = format!("{column}_kind");
    let value: Option<V> = row.try_get(column).map_err(|e| read(e, column))?;
    let kind: Option<String> = row
        .try_get(kind_column.as_str())
        .map_err(|e| read(e, &kind_column))?;
    match (value, kind) {
        (None, None) => Ok(None),
        (Some(value), Some(kind)) => {
            let value = convert(value).ok_or_else(|| {
                AppError::database(format!("{column} is out of range for its type"))
            })?;
            let origin_column = format!("{column}_origin");
            let as_of_column = format!("{column}_as_of");
            let origin: Option<String> = row
                .try_get(origin_column.as_str())
                .map_err(|e| read(e, &origin_column))?;
            let as_of: Option<NaiveDate> = row
                .try_get(as_of_column.as_str())
                .map_err(|e| read(e, &as_of_column))?;
            Ok(Some(ProvenancedValue::new(
                value,
                MetricProvenance {
                    kind: MeasurementKind::parse_lenient(&kind),
                    origin,
                    as_of,
                },
            )))
        }
        (Some(_), None) => Err(AppError::database(format!(
            "{column} is stored without its kind (measured or estimated)"
        ))),
        (None, Some(_)) => Err(AppError::database(format!(
            "{kind_column} is stored without a {column} value"
        ))),
    }
}

/// Emit the [`UserPhysiologicalProfileRepository`] and [`DossierRepository`]
/// implementations for one backend type. The bodies are written once here;
/// each backend's shell invokes it with its own type, and sqlx resolves the
/// driver from `self.pool()` per expansion. The dossier composer issues no
/// SQL of its own: it reads through the other repository traits the backend
/// already implements.
macro_rules! impl_user_physiological_profile_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl UserPhysiologicalProfileRepository for $ty {
            async fn upsert_user_physiological_profile(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                profile: &UserPhysiologicalProfile,
            ) -> AppResult<()> {
                if profile.user_id != user_id {
                    return Err(AppError::invalid_input(
                        "profile.user_id does not match the user_id passed to upsert_user_physiological_profile",
                    ));
                }
                let binds = profile_binds(profile)?;
                sqlx::query(UPSERT_PHYSIOLOGICAL_PROFILE_SQL)
                    .bind(UserId::from_uuid(user_id))
                    .bind(tenant_id)
                    .bind(profile.vo2_max)
                    .bind(profile.resting_hr.map(i32::from))
                    .bind(profile.max_hr.map(i32::from))
                    .bind(profile.lactate_threshold_percentage)
                    .bind(profile.age.map(i32::from))
                    .bind(profile.weight)
                    .bind(&binds.fitness_level)
                    .bind(&binds.primary_sport)
                    .bind(profile.training_experience_years.map(i32::from))
                    .bind(binds.ftp_watts)
                    .bind(profile.threshold_pace_sec_per_km)
                    .bind(binds.hr_zones)
                    .bind(binds.power_zones)
                    .bind(profile.threshold_hr.map(i32::from))
                    .bind(binds.critical_power_watts.value)
                    .bind(binds.critical_power_watts.kind)
                    .bind(binds.critical_power_watts.origin)
                    .bind(binds.critical_power_watts.as_of)
                    .bind(binds.w_prime_joules.value)
                    .bind(binds.w_prime_joules.kind)
                    .bind(binds.w_prime_joules.origin)
                    .bind(binds.w_prime_joules.as_of)
                    .bind(binds.critical_speed_mps.value)
                    .bind(binds.critical_speed_mps.kind)
                    .bind(binds.critical_speed_mps.origin)
                    .bind(binds.critical_speed_mps.as_of)
                    .bind(binds.d_prime_meters.value)
                    .bind(binds.d_prime_meters.kind)
                    .bind(binds.d_prime_meters.origin)
                    .bind(binds.d_prime_meters.as_of)
                    .bind(profile.transport_policy.is_first_party_only())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("upsert_user_physiological_profile: {e}"))
                    })?;
                Ok(())
            }

            async fn get_user_physiological_profile(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Option<UserPhysiologicalProfile>> {
                let row = sqlx::query(GET_PHYSIOLOGICAL_PROFILE_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("get_user_physiological_profile: {e}"))
                    })?;
                row.map(|r| profile_from_row(&r, user_id)).transpose()
            }
        }

        #[async_trait::async_trait]
        impl DossierRepository for $ty {
            async fn compose_dossier(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                readable: TransportPolicy,
                admit: &(dyn Fn(TransportPolicy) -> bool + Sync),
            ) -> AppResult<Dossier> {
                let physiology = self
                    .get_user_physiological_profile(tenant_id, user_id)
                    .await?
                    .filter(|profile| admit(profile.transport_policy));
                let hr_zones = physiology.as_ref().and_then(|p| p.hr_zones);
                let power_zones = physiology.as_ref().and_then(|p| p.power_zones);

                let goals = self.get_goals(user_id).await.unwrap_or_default();

                let raw_profile = match self.get_profile(user_id).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(error = %e, user_id = %user_id, "get_profile failed; degrading to empty profile");
                        None
                    }
                };
                let nutrition = raw_profile
                    .as_ref()
                    .and_then(|v| v.get("nutrition").cloned());
                let equipment = raw_profile
                    .as_ref()
                    .and_then(|v| v.get("equipment").cloned());

                // Per-user pillar context, grouped into pillar / north-star / medical
                // buckets. Best-effort — facts are an enhancement, not a hard
                // dependency of the dossier. The general read is recency-bounded
                // (LIMIT, ORDER BY updated_at DESC); two guaranteed fetches sit
                // alongside it so the facts that matter most cannot be evicted from
                // that window — group_facts dedupes the overlap.
                //
                // By kind: Medical and NorthStar, whichever source wrote them (a
                // agent-authored medical flag is not `source=onboarding`).
                //
                // By source: everything a guided interview captured. Kind cannot
                // protect these — a calibration answer about recovery speed is a
                // `preference`, indistinguishable from any chat-inferred preference —
                // so a chatty athlete used to lose their whole calibration set inside
                // the 40-fact window within weeks, which is the failure that made the
                // interview feel like theatre. Superseded rows are excluded by the
                // query, so a re-run's expired answers do not refill the bundle.
                //
                // A read that fails degrades to an empty set so the dossier still
                // renders, but it warns first: a bind or decode fault on these queries
                // strips medical flags and every interview answer from the agent's
                // context, and the agent keeps prescribing either way.
                let user = user_id.to_string();
                let mut facts = match self
                    .list_user_facts(tenant_id, &user, None, None, FACT_BUNDLE_LIMIT, readable)
                    .await
                {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::warn!(error = %e, user_id = %user_id, "recency-window fact read failed; dossier degrades to no general facts");
                        Vec::new()
                    }
                };
                for kind in [FactKind::Medical, FactKind::NorthStar] {
                    match self
                        .list_user_facts(
                            tenant_id,
                            &user,
                            None,
                            Some(kind),
                            FACT_BUNDLE_LIMIT,
                            readable,
                        )
                        .await
                    {
                        Ok(guaranteed) => facts.extend(guaranteed),
                        Err(e) => {
                            tracing::warn!(error = %e, user_id = %user_id, fact_kind = kind.as_str(), "guaranteed by-kind fact read failed; dossier degrades without this kind");
                        }
                    }
                }
                match self
                    .list_user_facts_by_source(
                        tenant_id,
                        &user,
                        FactSource::Onboarding,
                        FACT_BUNDLE_LIMIT,
                        readable,
                    )
                    .await
                {
                    Ok(onboarding) => facts.extend(onboarding),
                    Err(e) => {
                        tracing::warn!(error = %e, user_id = %user_id, fact_source = FactSource::Onboarding.as_str(), "guaranteed by-source fact read failed; dossier degrades without interview answers");
                    }
                }
                facts.retain(|fact| admit(fact.transport_policy));
                let buckets = group_facts(&facts, Utc::now());

                Ok(Dossier {
                    user_id,
                    tenant_id: tenant_id.as_uuid(),
                    physiology,
                    hr_zones,
                    power_zones,
                    goals,
                    nutrition,
                    equipment,
                    pillars: buckets.pillars,
                    north_star: buckets.north_star,
                    medical: buckets.medical,
                })
            }
        }
    };
}
pub(crate) use impl_user_physiological_profile_repository;
