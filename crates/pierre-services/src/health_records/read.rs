// ABOUTME: Reading the athlete's health records: one reader for their own screens, Dravr's agent and a coach's read
// ABOUTME: Every row gated by its own tier (tier_readable) and review state; a model or coach gets the stripped copy, never the resource

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Reading health records
//!
//! Who reads what (owl phase 5, slice 2):
//!
//! - **the athlete** on their own screens reads every served record with its
//!   resource as read ([`RecordView::Owner`]);
//! - **Dravr's agent** reads the served records the tier rule lets it
//!   ([`ai_scope::tier_readable`]), the free-text-stripped copy only
//!   ([`RecordView::Served`]); each one left out is tallied
//!   ([`ai_scope::withhold_tier`]) so the result carries the withheld note;
//! - **a human coach's read** reads the same, a T2 record only while the
//!   athlete's health-records grant to that coach stands
//!   ([`CoachReach::health_records`]), and no T3 record at all: no per-coach
//!   T3 grant is read yet.
//!
//! A served record is one read as it is, or one the athlete confirmed; a
//! pending record reaches only the athlete's review queue, a rejected one
//! no one. Glucose is read from both stores: barrage's timed readings (the
//! `BloodGlucose` series) and each day's `health_snapshots.blood_glucose`.

use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use dravr_barrage::Tier;
use dravr_riviere::SeriesType;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{StoredHealthMetrics, TenantId};
use pierre_database::repositories::{
    ClinicalSelection, RecordWindow, StoredClinicalRecord, StoredSelfReport,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::continuous_series::{continuous_series, SeriesRead};
use crate::stored_health::{self, RowProjection, SourceFilter};

/// The kinds of health record a read asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// Lab results: FHIR Observations, a lab document's among them.
    Labs,
    /// Every other clinical record: conditions, medications, reports.
    Clinical,
    /// Self-measured glucose: the timed readings and each day's value.
    Glucose,
    /// Cycle tracking and the other self-reports.
    SelfReports,
}

impl RecordKind {
    /// The kind `name` spells.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "labs" => Some(Self::Labs),
            "clinical" => Some(Self::Clinical),
            "glucose" => Some(Self::Glucose),
            "self_reports" => Some(Self::SelfReports),
            _ => None,
        }
    }
}

/// Which copy of a clinical record a read serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordView {
    /// The athlete's own read: the resource as read.
    Owner,
    /// Anyone else's: the copy with every free-text field removed.
    Served,
}

/// What a coach's read may reach of the athlete's records, `None` on any
/// other read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoachReach {
    /// The athlete's health-records grant to this coach stands and has not
    /// lapsed.
    pub health_records: bool,
}

/// One clinical record as a read serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ServedClinicalRecord {
    /// The record's id.
    pub id: String,
    /// The FHIR `resourceType`.
    pub resource_type: String,
    /// Its tier (`T2`, `T3`).
    pub tier: String,
    /// Why a code raised it to T3, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity_class: Option<String>,
    /// When it was effective.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_at: Option<DateTime<Utc>>,
    /// The source it came from (an import's key, or `lab_document`).
    pub source: String,
    /// The resource as read (the athlete's read) or without free text.
    #[schemars(with = "serde_json::Value")]
    pub resource: Value,
    /// Its codes and numbers, for an Observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub flat: Option<Value>,
    /// How it came from its file (`verbatim`, `parsed`, `extracted`).
    pub derivation: String,
    /// The engine that read it from a document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_by: Option<String>,
}

/// One self-report as a read serves it.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ServedSelfReport {
    /// The record's id.
    pub id: String,
    /// What it records.
    pub kind: String,
    /// When it starts.
    pub start: DateTime<Utc>,
    /// When it ends.
    pub end: DateTime<Utc>,
    /// A coded value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_code: Option<String>,
    /// A measured value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Its unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The start of a cycle.
    pub cycle_start: bool,
    /// The source it came from.
    pub source: String,
}

/// One timed glucose reading.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GlucoseReadingOut {
    /// When it was measured.
    pub at: DateTime<Utc>,
    /// The reading, mg/dL.
    pub mg_dl: f64,
    /// The source it came from.
    pub source: String,
}

/// One day's glucose, as a source synced it.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GlucoseDay {
    /// The day.
    pub date: NaiveDate,
    /// The day's reading, mg/dL.
    pub mg_dl: f64,
    /// The source it came from.
    pub source: String,
}

/// What one read of the athlete's health records serves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, schemars::JsonSchema)]
pub struct HealthRecordsPage {
    /// Clinical records, for `labs` and `clinical`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<ServedClinicalRecord>,
    /// Self-reports, for `self_reports`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub self_reports: Vec<ServedSelfReport>,
    /// Timed glucose readings, for `glucose`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub readings: Vec<GlucoseReadingOut>,
    /// Each day's glucose, for `glucose`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub daily: Vec<GlucoseDay>,
}

/// A stored tier, as barrage serializes it.
fn stored_tier(tier: &str) -> AppResult<Tier> {
    serde_json::from_value(Value::String(tier.to_owned()))
        .map_err(|e| AppError::internal(format!("stored health record tier {tier}: {e}")))
}

/// Whether a `tier` record reaches this read, tallied when the tier rule
/// leaves it out. A coach's read never reaches T3, and T2 only under their
/// health-records grant.
fn reaches(tier: Tier, coach: Option<CoachReach>) -> bool {
    let granted = match (coach, tier) {
        (None, _) | (Some(_), Tier::T0 | Tier::T1) => true,
        (Some(reach), Tier::T2) => reach.health_records,
        (Some(_), Tier::T3) => false,
    };
    if !granted {
        return false;
    }
    if ai_scope::tier_readable(tier) {
        return true;
    }
    ai_scope::withhold_tier(tier);
    false
}

/// A stored JSON column.
fn stored_json(text: &str) -> AppResult<Value> {
    serde_json::from_str(text)
        .map_err(|e| AppError::internal(format!("stored health record JSON: {e}")))
}

/// One clinical record as `view` serves it.
fn served_clinical(
    record: StoredClinicalRecord,
    view: RecordView,
) -> AppResult<ServedClinicalRecord> {
    let resource = match view {
        RecordView::Owner => &record.resource,
        RecordView::Served => &record.stripped,
    };
    Ok(ServedClinicalRecord {
        resource: stored_json(resource)?,
        flat: record.flat.as_deref().map(stored_json).transpose()?,
        id: record.id,
        resource_type: record.resource_type,
        tier: record.tier,
        sensitivity_class: record.sensitivity_class,
        effective_at: record.effective_at,
        source: record.provider,
        derivation: record.derivation,
        read_by: record.read_by,
    })
}

/// One self-report as served.
fn served_self_report(report: StoredSelfReport) -> ServedSelfReport {
    ServedSelfReport {
        id: report.id,
        kind: report.kind,
        start: report.start_at,
        end: report.end_at,
        value_code: report.value_code,
        value: report.value,
        unit: report.unit,
        cycle_start: report.cycle_start,
        source: report.provider,
    }
}

/// The served clinical records of `kind` (labs or the rest) the read
/// reaches.
async fn clinical_page(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, &TenantId),
    observations: bool,
    window: RecordWindow,
    (view, coach): (RecordView, Option<CoachReach>),
) -> AppResult<Vec<ServedClinicalRecord>> {
    let stored = repos
        .health_records
        .list_clinical_records(
            tenant,
            user_id,
            ClinicalSelection::Served {
                observations: Some(observations),
            },
            window,
        )
        .await?;
    let mut served = Vec::with_capacity(stored.len());
    for record in stored {
        if reaches(stored_tier(&record.tier)?, coach) {
            served.push(served_clinical(record, view)?);
        }
    }
    Ok(served)
}

/// The athlete's glucose in `range`: the timed readings, then each day's.
///
/// `project` cuts each day's body row as the reader may see it (a coach's
/// read keeps only what their grant names); the readings follow the same
/// tier rule, reading by reading.
async fn glucose_page(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    (user_id, tenant): (Uuid, &TenantId),
    range: (DateTime<Utc>, DateTime<Utc>),
    (coach, project): (Option<CoachReach>, RowProjection<StoredHealthMetrics>),
) -> AppResult<HealthRecordsPage> {
    let mut page = HealthRecordsPage::default();
    let series = continuous_series(
        repos,
        terms,
        user_id,
        tenant,
        SeriesType::BloodGlucose,
        range,
        SeriesRead::Points,
    )
    .await?;
    for source in series.records {
        for point in source.points {
            if reaches(Tier::T2, coach) {
                page.readings.push(GlucoseReadingOut {
                    at: point.timestamp,
                    mg_dl: point.value,
                    source: source.provider.clone(),
                });
            }
        }
    }
    let days = stored_health::merged_body(
        repos,
        terms,
        user_id,
        tenant,
        range,
        SourceFilter::All,
        project,
    )
    .await?;
    page.daily = days
        .records
        .into_iter()
        .filter_map(|day| {
            day.record.blood_glucose.map(|mg_dl| GlucoseDay {
                date: day.record.date,
                mg_dl,
                source: day.record.source_name,
            })
        })
        .collect();
    Ok(page)
}

/// One read of the athlete's health records.
#[derive(Clone, Copy)]
pub struct RecordsRead<'read> {
    /// The repositories.
    pub repos: &'read RepositoryRegistry,
    /// The provider terms every stored read is governed by.
    pub terms: &'read dyn ProviderTerms,
    /// The athlete.
    pub user_id: Uuid,
    /// Their tenant.
    pub tenant: &'read TenantId,
    /// Which copy a clinical record is served as.
    pub view: RecordView,
    /// What a coach's read reaches; `None` on any other read.
    pub coach: Option<CoachReach>,
    /// How a day's body row is cut for this reader.
    pub project: RowProjection<StoredHealthMetrics>,
}

impl fmt::Debug for RecordsRead<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordsRead")
            .field("user_id", &self.user_id)
            .field("view", &self.view)
            .field("coach", &self.coach)
            .finish_non_exhaustive()
    }
}

impl RecordsRead<'_> {
    /// The records of `kind` in `window` this read reaches.
    ///
    /// # Errors
    ///
    /// The store's error.
    pub async fn read(
        &self,
        kind: RecordKind,
        window: RecordWindow,
    ) -> AppResult<HealthRecordsPage> {
        let owner = (self.user_id, self.tenant);
        match kind {
            RecordKind::Labs | RecordKind::Clinical => Ok(HealthRecordsPage {
                records: clinical_page(
                    self.repos,
                    owner,
                    kind == RecordKind::Labs,
                    window,
                    (self.view, self.coach),
                )
                .await?,
                ..HealthRecordsPage::default()
            }),
            RecordKind::SelfReports => {
                let stored = self
                    .repos
                    .health_records
                    .list_self_reports(self.tenant, self.user_id, window)
                    .await?;
                // Every self-report is T3.
                let self_reports = stored
                    .into_iter()
                    .filter(|_| reaches(Tier::T3, self.coach))
                    .map(served_self_report)
                    .collect();
                Ok(HealthRecordsPage {
                    self_reports,
                    ..HealthRecordsPage::default()
                })
            }
            RecordKind::Glucose => {
                let until = window.until.unwrap_or_else(Utc::now);
                let since = window
                    .since
                    .unwrap_or_else(|| until - chrono::Duration::days(GLUCOSE_DEFAULT_DAYS));
                // Two stored reads in one future: boxed, so every caller's
                // future stays small.
                Box::pin(glucose_page(
                    self.repos,
                    self.terms,
                    owner,
                    (since, until),
                    (self.coach, self.project),
                ))
                .await
            }
        }
    }
}

/// How far back a glucose read with no `since` reaches.
pub const GLUCOSE_DEFAULT_DAYS: i64 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_read_by_its_name() {
        for (name, kind) in [
            ("labs", RecordKind::Labs),
            ("clinical", RecordKind::Clinical),
            ("glucose", RecordKind::Glucose),
            ("self_reports", RecordKind::SelfReports),
        ] {
            assert_eq!(RecordKind::parse(name), Some(kind), "bug: {name}");
        }
        assert_eq!(RecordKind::parse("notes"), None, "bug: an unknown kind");
    }

    #[test]
    fn a_stored_tier_reads_back_as_barrage_wrote_it() {
        assert_eq!(stored_tier("T2").ok(), Some(Tier::T2), "bug: T2");
        assert_eq!(stored_tier("T3").ok(), Some(Tier::T3), "bug: T3");
        assert!(stored_tier("t9").is_err(), "bug: an unknown tier read");
    }

    #[test]
    fn a_coach_never_reaches_t3_and_t2_only_under_the_grant() {
        let granted = Some(CoachReach {
            health_records: true,
        });
        let not_granted = Some(CoachReach {
            health_records: false,
        });
        assert!(!reaches(Tier::T3, granted), "bug: a coach reached T3");
        assert!(!reaches(Tier::T2, not_granted), "bug: T2 without the grant");
        // Outside any gate (the athlete's own read) every tier is read.
        assert!(
            reaches(Tier::T2, granted),
            "bug: the grant was not honoured"
        );
        assert!(reaches(Tier::T3, None), "bug: the athlete's own T3 read");
    }
}
