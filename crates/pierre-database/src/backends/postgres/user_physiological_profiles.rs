// ABOUTME: PostgreSQL implementation of UserPhysiologicalProfileRepository + DossierRepository (Endurance Phase 1)
// ABOUTME: Mirrors crates/pierre-database/src/database/user_physiological_profiles.rs for the Postgres tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::zones::{HrZoneSet, PowerZoneSet};
use pierre_core::models::{Dossier, TenantId, UserPhysiologicalProfile};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{
    DossierRepository, ProfileRepository, UserPhysiologicalProfileRepository,
};

#[async_trait]
impl UserPhysiologicalProfileRepository for PostgresDatabase {
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
        let hr_zones_json: Option<Value> = profile
            .hr_zones
            .as_ref()
            .map(|z| {
                serde_json::to_value(z)
                    .map_err(|e| AppError::database(format!("serialize hr_zones: {e}")))
            })
            .transpose()?;
        let power_zones_json: Option<Value> = profile
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

        sqlx::query(
            r"
            INSERT INTO user_physiological_profiles (
                user_id, tenant_id, vo2_max, resting_hr, max_hr,
                lactate_threshold_percentage, age, weight, fitness_level,
                primary_sport, training_experience_years, ftp_watts,
                threshold_pace_sec_per_km, hr_zones_json, power_zones_json,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW(), NOW())
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
                updated_at = NOW()
            ",
        )
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .bind(profile.vo2_max)
        .bind(profile.resting_hr.map(i32::from))
        .bind(profile.max_hr.map(i32::from))
        .bind(profile.lactate_threshold_percentage)
        .bind(profile.age.map(i32::from))
        .bind(profile.weight)
        .bind(&fitness_level)
        .bind(&primary_sport)
        .bind(profile.training_experience_years.map(i32::from))
        .bind(profile.ftp_watts.and_then(|v| i32::try_from(v).ok()))
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
            WHERE tenant_id = $1 AND user_id = $2
            LIMIT 1
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
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

        let resting_hr: Option<i32> = row.try_get("resting_hr").ok();
        let max_hr: Option<i32> = row.try_get("max_hr").ok();
        let age: Option<i32> = row.try_get("age").ok();
        let training_years: Option<i32> = row.try_get("training_experience_years").ok();
        let ftp_watts_db: Option<i32> = row.try_get("ftp_watts").ok();

        let hr_zones_json: Option<Value> = row.try_get("hr_zones_json").ok();
        let power_zones_json: Option<Value> = row.try_get("power_zones_json").ok();
        let hr_zones: Option<HrZoneSet> = hr_zones_json
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| AppError::database(format!("parse hr_zones_json: {e}")))?;
        let power_zones: Option<PowerZoneSet> = power_zones_json
            .map(serde_json::from_value)
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
impl DossierRepository for PostgresDatabase {
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
