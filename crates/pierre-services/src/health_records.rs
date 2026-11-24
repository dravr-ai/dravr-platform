// ABOUTME: Barrage's T2–T3 records as the platform stores them: glucose in mg/dL, self-reports and clinical records as rows
// ABOUTME: One mapping for the archive sink and the lab-document reader; storing is never gated, reading is (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Storing health records
//!
//! JF 2026-11-20, "Store, gate reads": every import keeps the T2–T3 records
//! dravr-barrage reads for the athlete — self-measured glucose, cycle-tracking
//! self-reports, FHIR R4 clinical records — and the tier consent decides only
//! who else reads them. This module is the one place those records become
//! the platform's rows:
//!
//! - glucose goes to riviere's `BloodGlucose` series in mg/dL
//!   ([`IntradaySeries::blood_glucose`]), converted from mmol/L where the
//!   export wrote that; any other unit is dropped and counted;
//! - self-reports and clinical records go to their tables
//!   ([`HealthRecordRepository`]), each keyed by its barrage id, so a
//!   re-import replaces its rows and never an athlete's review decision.

/// Reading them: one reader for the athlete, Dravr's agent and a coach's read.
pub mod read;

use dravr_barrage::{
    ClinicalResource, GlucoseReading, Locator, ReadingEngine, SelfReport, WellnessRecords,
};
use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{ClinicalRecordWrite, RecordFiling, SelfReportWrite};
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use serde_json::Value;

use crate::wellness_store::{IntradaySeries, WellnessStore};

/// mg/dL per mmol/L of glucose: its molar mass, 180.156 g/mol, over ten.
pub const GLUCOSE_MG_DL_PER_MMOL_L: f64 = 18.0156;

/// The unit riviere's `BloodGlucose` series is kept in.
const MG_DL: &str = "mg/dL";

/// The unit a glucose reading may arrive in that is converted.
const MMOL_L: &str = "mmol/L";

/// A glucose reading in mg/dL, or `None` for a unit neither stored nor
/// converted.
#[must_use]
pub fn glucose_mg_dl(reading: &GlucoseReading) -> Option<f64> {
    match reading.unit.as_str() {
        MG_DL => Some(reading.value),
        MMOL_L => Some(reading.value * GLUCOSE_MG_DL_PER_MMOL_L),
        _ => None,
    }
    .filter(|value| value.is_finite() && *value > 0.0)
}

/// What storing an import's sensitive records wrote and dropped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SensitiveStored {
    /// Glucose readings written to the series.
    pub glucose: usize,
    /// Glucose readings dropped: a unit neither stored nor converted.
    pub glucose_dropped: usize,
    /// Self-report rows written.
    pub self_reports: u64,
    /// Clinical record rows written.
    pub clinical: u64,
}

/// A serde enum's wire name, as barrage serializes it.
fn wire_name<T: Serialize>(value: &T) -> AppResult<String> {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => Ok(name),
        Ok(other) => Ok(other.to_string()),
        Err(e) => Err(AppError::internal(format!("health record field: {e}"))),
    }
}

/// A JSON value as stored.
fn json_text<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|e| AppError::internal(format!("health record JSON: {e}")))
}

/// One self-report as stored.
///
/// # Errors
///
/// Never in practice: barrage's kinds serialize as strings.
pub fn self_report_write(report: &SelfReport) -> AppResult<SelfReportWrite> {
    Ok(SelfReportWrite {
        record_id: report.id.clone(),
        kind: wire_name(&report.kind)?,
        start_at: report.start,
        end_at: report.end,
        value_code: report.value_code.clone(),
        value: report.value,
        unit: report.unit.clone(),
        cycle_start: report.cycle_start,
        protection_used: report.protection_used,
        legacy_id: report.legacy_id.clone(),
    })
}

/// The instant a FHIR date or date-time names: RFC 3339, or a bare date at
/// midnight UTC.
fn fhir_instant(text: &str) -> Option<Timestamp> {
    Timestamp::parse_from_rfc3339(text).ok().or_else(|| {
        CivilDate::parse_from_str(text, "%Y-%m-%d")
            .ok()
            .and_then(|day| day.at_midnight().and_utc().ok())
    })
}

/// The FHIR elements a resource is dated by, in the order they are tried.
const FHIR_DATE_ELEMENTS: [&str; 7] = [
    "effectiveDateTime",
    "issued",
    "recordedDate",
    "onsetDateTime",
    "authoredOn",
    "occurrenceDateTime",
    "date",
];

/// When a clinical record was effective: the flattened Observation's date,
/// else the first date element the resource carries.
#[must_use]
pub fn effective_at(record: &ClinicalResource) -> Option<Timestamp> {
    record
        .flat
        .as_ref()
        .and_then(|flat| flat.effective_at.as_deref())
        .and_then(fhir_instant)
        .or_else(|| {
            FHIR_DATE_ELEMENTS.iter().find_map(|element| {
                record
                    .resource
                    .get(element)
                    .and_then(Value::as_str)
                    .and_then(fhir_instant)
            })
        })
}

/// One clinical record as stored, read from the lab document
/// `lab_document_id` when it was.
///
/// # Errors
///
/// When a field does not serialize, which barrage's types rule out.
pub fn clinical_write(
    record: &ClinicalResource,
    lab_document_id: Option<&str>,
) -> AppResult<ClinicalRecordWrite> {
    Ok(ClinicalRecordWrite {
        record_id: record.id.clone(),
        resource_type: record.resource_type.clone(),
        tier: wire_name(&record.tier)?,
        sensitivity_class: record
            .sensitivity_class
            .as_ref()
            .map(wire_name)
            .transpose()?,
        fhir_version: record.fhir_version.clone(),
        resource: json_text(&record.resource)?,
        stripped: json_text(&record.stripped)?,
        provenance: json_text(&record.provenance)?,
        flat: record.flat.as_ref().map(json_text).transpose()?,
        derivation: wire_name(&record.derivation)?,
        review_state: wire_name(&record.review_state)?,
        checks: json_text(&record.checks)?,
        locator: record.locator.map(Locator::to_text),
        read_by: record.read_by.as_ref().map(ReadingEngine::label),
        effective_at: effective_at(record),
        lab_document_id: lab_document_id.map(str::to_owned),
        legacy_id: record.legacy_id.clone(),
    })
}

/// Store the T2–T3 records `wellness` holds under `filing`: glucose in the
/// series of `filing`'s data source, self-reports and clinical records in
/// their tables.
///
/// # Errors
///
/// The store's error when a write fails.
pub async fn store_sensitive(
    repos: &RepositoryRegistry,
    filing: RecordFiling<'_>,
    wellness: &WellnessRecords,
) -> AppResult<SensitiveStored> {
    let readings: Vec<_> = wellness
        .glucose
        .iter()
        .filter_map(|reading| glucose_mg_dl(reading).map(|mg_dl| (reading.at, mg_dl)))
        .collect();
    let mut stored = SensitiveStored {
        glucose: readings.len(),
        glucose_dropped: wellness.glucose.len() - readings.len(),
        ..SensitiveStored::default()
    };
    WellnessStore::new(repos)
        .store_series(
            filing.data_source_id,
            IntradaySeries::blood_glucose(readings),
        )
        .await?;
    let reports = wellness
        .self_reports
        .iter()
        .map(self_report_write)
        .collect::<AppResult<Vec<_>>>()?;
    stored.self_reports = repos
        .health_records
        .upsert_self_reports(filing, &reports)
        .await?;
    let clinical = wellness
        .clinical
        .iter()
        .map(|record| clinical_write(record, None))
        .collect::<AppResult<Vec<_>>>()?;
    stored.clinical = repos
        .health_records
        .upsert_clinical_records(filing, &clinical)
        .await?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glucose(value: f64, unit: &str) -> GlucoseReading {
        GlucoseReading {
            id: "g".to_owned(),
            at: Timestamp::from_epoch_seconds(1_700_000_000).unwrap(),
            value,
            unit: unit.to_owned(),
            loinc: "2339-0".to_owned(),
        }
    }

    #[test]
    fn mmol_per_litre_is_stored_as_mg_per_decilitre() {
        let converted = glucose_mg_dl(&glucose(5.5, "mmol/L")).unwrap();
        assert!(
            (converted - 99.0858).abs() < 1e-9,
            "bug: 5.5 mmol/L read as {converted} mg/dL"
        );
        assert_eq!(
            glucose_mg_dl(&glucose(99.0, "mg/dL")),
            Some(99.0),
            "bug: a mg/dL reading was changed"
        );
    }

    #[test]
    fn any_other_unit_or_an_impossible_value_is_dropped() {
        assert_eq!(
            glucose_mg_dl(&glucose(5.5, "g/L")),
            None,
            "bug: an unconverted unit was stored"
        );
        assert_eq!(
            glucose_mg_dl(&glucose(-1.0, "mg/dL")),
            None,
            "bug: a negative reading was stored"
        );
        assert_eq!(
            glucose_mg_dl(&glucose(f64::NAN, "mmol/L")),
            None,
            "bug: NaN was stored"
        );
    }

    #[test]
    fn a_fhir_date_or_date_time_is_read_as_an_instant() {
        assert_eq!(
            fhir_instant("2026-03-04"),
            Timestamp::parse_from_rfc3339("2026-03-04T00:00:00Z").ok(),
            "bug: a bare date"
        );
        assert_eq!(
            fhir_instant("2026-03-04T08:30:00-05:00"),
            Timestamp::parse_from_rfc3339("2026-03-04T13:30:00Z").ok(),
            "bug: a date-time with an offset"
        );
        assert_eq!(fhir_instant("March"), None, "bug: prose read as a date");
    }
}
