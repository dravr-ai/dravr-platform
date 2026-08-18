// ABOUTME: Endurance athlete Dossier aggregate composed at read time from physiology, goals, zones, nutrition, equipment
// ABOUTME: Backs GET /api/v1/endurance/dossier — never persisted as a single row, always assembled per-request
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::zones::{HrZoneSet, PowerZoneSet};
use super::{Pillar, UserPhysiologicalProfile};

/// A single pillar-context fact projected onto the dossier.
///
/// This is a read-time projection of a `pierre_memory::UserFact` — the subset
/// the per-user OKF bundle needs to render, plus a computed freshness flag.
/// `kind` and `source` are carried as their stable string slugs to keep this
/// type free of a `pierre-memory` dependency. The `object` field is
/// user-authored free text and must be treated as untrusted at render time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DossierFact {
    /// `FactKind` slug (e.g. `goal`, `injury`, `north_star`, `medical`).
    pub kind: String,
    /// Subject phrase (typically "you").
    pub subject: String,
    /// Predicate phrase.
    pub predicate: String,
    /// Object phrase — UNTRUSTED user free text.
    pub object: String,
    /// Extractor/author confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// `FactSource` slug (onboarding / conversation / device / coach).
    pub source: String,
    /// When the fact was last touched.
    pub updated_at: DateTime<Utc>,
    /// Freshness horizon, if any.
    pub valid_until: Option<DateTime<Utc>>,
    /// Computed at compose time: `valid_until` is in the past.
    pub stale: bool,
}

/// Endurance athlete dossier.
///
/// Composed at read time from the per-tenant rows of:
///
/// - `user_physiological_profiles` → [`UserPhysiologicalProfile`] (present
///   once `set_physiology` has written the row; `None` until then)
/// - `user_profiles` JSON column → free-form goal entries (`goals`)
/// - `user_physiological_profiles.hr_zones_json` /
///   `power_zones_json` → typed [`HrZoneSet`] / [`PowerZoneSet`]
/// - `user_profiles` JSON column → optional nutrition snapshot
///   (free-form JSON)
/// - `user_profiles` JSON column → optional equipment snapshot
///   (free-form JSON, same shape as the nutrition snapshot)
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
    ///
    /// `None` until the athlete's measurements are written by the
    /// `set_physiology` tool, which is the only production writer of
    /// `user_physiological_profiles`.
    pub physiology: Option<UserPhysiologicalProfile>,

    /// Heart-rate zone definitions.
    ///
    /// `None` until `set_physiology` derives them, which it does as soon as
    /// both a resting and a maximum heart rate are on the profile.
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

    /// Per-user pillar context: durable facts grouped by health pillar. Empty
    /// until the user has answered onboarding or accumulated conversation facts.
    /// `BTreeMap` keeps JSON/markdown rendering order deterministic.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pillars: BTreeMap<Pillar, Vec<DossierFact>>,

    /// The user's North Star — one to three core life motivations orienting the
    /// pillars. Rendered as the index of the OKF bundle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub north_star: Vec<DossierFact>,

    /// Medical / PAR-Q flags the coach must heed. Gated separately from pillar
    /// facts so raw medical detail can be withheld per privacy policy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub medical: Vec<DossierFact>,
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
            pillars: BTreeMap::new(),
            north_star: Vec::new(),
            medical: Vec::new(),
        }
    }
}
