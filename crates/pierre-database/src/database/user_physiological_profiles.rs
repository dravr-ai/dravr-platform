// ABOUTME: SQLite implementation of UserPhysiologicalProfileRepository + DossierRepository (Endurance Phase 1)
// ABOUTME: Backs GET /api/v1/endurance/latest and GET /api/v1/endurance/dossier on the SQLite tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::zones::{HrZoneSet, PowerZoneSet};
use pierre_core::models::{Dossier, TenantId, UserPhysiologicalProfile};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::{
    DossierRepository, ProfileRepository, UserPhysiologicalProfileRepository,
};

#[async_trait]
impl UserPhysiologicalProfileRepository for Database {
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
        let hr_zones_json = profile
            .hr_zones
            .as_ref()
            .map(|z| {
                serde_json::to_string(z)
                    .map_err(|e| AppError::database(format!("Failed to serialize hr_zones: {e}")))
            })
            .transpose()?;
        let power_zones_json = profile
            .power_zones
            .as_ref()
            .map(|z| {
                serde_json::to_string(z).map_err(|e| {
                    AppError::database(format!("Failed to serialize power_zones: {e}"))
                })
            })
            .transpose()?;
        let fitness_level = serde_json::to_string(&profile.fitness_level)
            .map_err(|e| AppError::database(format!("Failed to serialize fitness_level: {e}")))?;
        let primary_sport = serde_json::to_string(&profile.primary_sport)
            .map_err(|e| AppError::database(format!("Failed to serialize primary_sport: {e}")))?;

        sqlx::query(
            r"
            INSERT INTO user_physiological_profiles (
                user_id, tenant_id, vo2_max, resting_hr, max_hr,
                lactate_threshold_percentage, age, weight, fitness_level,
                primary_sport, training_experience_years, ftp_watts,
                threshold_pace_sec_per_km, hr_zones_json, power_zones_json,
                created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(tenant_id, user_id) DO UPDATE SET
                vo2_max = excluded.vo2_max,
                resting_hr = excluded.resting_hr,
                max_hr = excluded.max_hr,
                lactate_threshold_percentage = excluded.lactate_threshold_percentage,
                age = excluded.age,
                weight = excluded.weight,
                fitness_level = excluded.fitness_level,
                primary_sport = excluded.primary_sport,
                training_experience_years = excluded.training_experience_years,
                ftp_watts = excluded.ftp_watts,
                threshold_pace_sec_per_km = excluded.threshold_pace_sec_per_km,
                hr_zones_json = excluded.hr_zones_json,
                power_zones_json = excluded.power_zones_json,
                updated_at = datetime('now')
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(profile.vo2_max)
        .bind(profile.resting_hr.map(i64::from))
        .bind(profile.max_hr.map(i64::from))
        .bind(profile.lactate_threshold_percentage)
        .bind(profile.age.map(i64::from))
        .bind(profile.weight)
        .bind(&fitness_level)
        .bind(&primary_sport)
        .bind(profile.training_experience_years.map(i64::from))
        .bind(profile.ftp_watts.map(i64::from))
        .bind(profile.threshold_pace_sec_per_km)
        .bind(hr_zones_json)
        .bind(power_zones_json)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("upsert_user_physiological_profile: {e}")))?;
        Ok(())
    }

    async fn get_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<UserPhysiologicalProfile>> {
        let row = sqlx::query(
            r"
            SELECT vo2_max, resting_hr, max_hr, lactate_threshold_percentage,
                   age, weight, fitness_level, primary_sport,
                   training_experience_years, ftp_watts,
                   threshold_pace_sec_per_km, hr_zones_json, power_zones_json
            FROM user_physiological_profiles
            WHERE tenant_id = ? AND user_id = ?
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("get_user_physiological_profile: {e}")))?;

        let Some(row) = row else { return Ok(None) };

        let fitness_level_str: String = row
            .try_get("fitness_level")
            .map_err(|e| AppError::database(format!("read fitness_level: {e}")))?;
        let fitness_level = serde_json::from_str(&fitness_level_str)
            .map_err(|e| AppError::database(format!("parse fitness_level: {e}")))?;
        let primary_sport_str: String = row
            .try_get("primary_sport")
            .map_err(|e| AppError::database(format!("read primary_sport: {e}")))?;
        let primary_sport = serde_json::from_str(&primary_sport_str)
            .map_err(|e| AppError::database(format!("parse primary_sport: {e}")))?;

        let resting_hr: Option<i64> = row.try_get("resting_hr").ok();
        let max_hr: Option<i64> = row.try_get("max_hr").ok();
        let age: Option<i64> = row.try_get("age").ok();
        let training_years: Option<i64> = row.try_get("training_experience_years").ok();
        let ftp_watts_db: Option<i64> = row.try_get("ftp_watts").ok();

        let hr_zones_json: Option<String> = row
            .try_get::<Option<String>, _>("hr_zones_json")
            .map_err(|e| AppError::database(format!("read hr_zones_json: {e}")))?;
        let power_zones_json: Option<String> = row
            .try_get::<Option<String>, _>("power_zones_json")
            .map_err(|e| AppError::database(format!("read power_zones_json: {e}")))?;
        let hr_zones: Option<HrZoneSet> = hr_zones_json
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| AppError::database(format!("parse hr_zones_json: {e}")))?;
        let power_zones: Option<PowerZoneSet> = power_zones_json
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| AppError::database(format!("parse power_zones_json: {e}")))?;

        Ok(Some(UserPhysiologicalProfile {
            user_id,
            vo2_max: row.try_get("vo2_max").ok(),
            resting_hr: resting_hr.and_then(|v| u16::try_from(v).ok()),
            max_hr: max_hr.and_then(|v| u16::try_from(v).ok()),
            lactate_threshold_percentage: row.try_get("lactate_threshold_percentage").ok(),
            age: age.and_then(|v| u16::try_from(v).ok()),
            weight: row.try_get("weight").ok(),
            fitness_level,
            primary_sport,
            training_experience_years: training_years.and_then(|v| u8::try_from(v).ok()),
            ftp_watts: ftp_watts_db.and_then(|v| u32::try_from(v).ok()),
            threshold_pace_sec_per_km: row.try_get("threshold_pace_sec_per_km").ok(),
            hr_zones,
            power_zones,
        }))
    }
}

#[async_trait]
impl DossierRepository for Database {
    async fn compose_dossier(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<Dossier> {
        let physiology = self
            .get_user_physiological_profile(tenant_id, user_id)
            .await?;
        let hr_zones = physiology.as_ref().and_then(|p| p.hr_zones);
        let power_zones = physiology.as_ref().and_then(|p| p.power_zones);

        let goals = self.get_goals(user_id).await.unwrap_or_default();

        let raw_profile = self.get_profile(user_id).await.unwrap_or(None);
        let nutrition = raw_profile
            .as_ref()
            .and_then(|v| v.get("nutrition").cloned());
        let equipment = raw_profile
            .as_ref()
            .and_then(|v| v.get("equipment").cloned());

        Ok(Dossier {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            physiology,
            hr_zones,
            power_zones,
            goals,
            nutrition,
            equipment,
        })
    }
}
