// ABOUTME: Endurance athlete Dossier aggregate composed at read time from physiology, goals, zones, nutrition, equipment
// ABOUTME: Backs GET /api/v1/endurance/dossier — never persisted as a single row, always assembled per-request
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::zones::{HrZoneSet, PowerZoneSet};
use super::UserPhysiologicalProfile;

/// Endurance athlete dossier.
///
/// Composed at read time from the per-tenant rows of:
///
/// - `user_physiological_profiles` → [`UserPhysiologicalProfile`] (always
///   present once the user has logged a profile; `None` until then)
/// - `user_profiles` JSON column → free-form goal entries (`goals`)
/// - `user_physiological_profiles.hr_zones_json` /
///   `power_zones_json` → typed [`HrZoneSet`] / [`PowerZoneSet`]
/// - `user_profiles` JSON column → optional nutrition snapshot
///   (currently free-form; will become typed once the nutrition Phase ships)
/// - `user_profiles` JSON column → optional equipment snapshot (same shape
///   as nutrition for now)
///
/// Per Endurance's Open Decision #1 the dossier is **not** persisted as
/// its own row. Composition lives in
/// [`pierre_database::DossierRepository`](https://docs.dravr.ai/db/dossier).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dossier {
    /// User this dossier belongs to.
    pub user_id: Uuid,

    /// Tenant scope.
    pub tenant_id: Uuid,

    /// Physiological profile slot (FTP, threshold pace, fitness level, etc.).
    /// `None` until the user has saved a profile.
    pub physiology: Option<UserPhysiologicalProfile>,

    /// Heart-rate zone definitions.
    /// `None` until the user has saved zones (manually or via threshold tests).
    pub hr_zones: Option<HrZoneSet>,

    /// Power zone definitions (cycling / running power).
    /// `None` for athletes without a power meter or saved FTP.
    pub power_zones: Option<PowerZoneSet>,

    /// Active goals (e.g. race target, weekly volume, weight). Free-form
    /// JSON until the typed goals model lands; the Endurance payload
    /// surfaces them as-is so coaches can read them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<Value>,

    /// Nutrition snapshot (preferences, dietary restrictions, hydration
    /// targets). Free-form JSON; `None` when the user has not configured
    /// nutrition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nutrition: Option<Value>,

    /// Equipment snapshot (shoes, bikes, sensors). Free-form JSON; `None`
    /// when the user has not configured equipment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment: Option<Value>,
}

impl Dossier {
    /// Build an empty dossier shell for a (`tenant_id`, `user_id`) pair.
    ///
    /// Useful as the starting point for the read-time composer when the
    /// user has no rows in the underlying tables — the endpoint returns a
    /// 200 with all slots `None` rather than a 404.
    #[must_use]
    pub const fn empty(tenant_id: Uuid, user_id: Uuid) -> Self {
        Self {
            user_id,
            tenant_id,
            physiology: None,
            hr_zones: None,
            power_zones: None,
            goals: Vec::new(),
            nutrition: None,
            equipment: None,
        }
    }
}
