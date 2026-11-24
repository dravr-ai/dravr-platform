// ABOUTME: Every import stores its T2–T3 records for the athlete; the tier consent gates who else reads them (carnet#1544)
// ABOUTME: Glucose lands as mg/dL, the athlete reads everything, Dravr's agent the stripped copy under consent, purges take it all

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Owl phase 5, slice 2. JF 2026-11-20, "Store, gate reads": an import keeps
//! the glucose, self-reports and clinical records barrage reads, whatever the
//! athlete allowed; the tier rule decides who else reads them.
//!
//! - the archive sink stores the three fields, glucose converted from mmol/L;
//! - the athlete's own read serves every record, the resource as read;
//! - Dravr's agent reads a T2 record only under `sensitive_t2`, a T3 one only
//!   under `sensitive_t3`, and always the free-text-stripped copy;
//! - retracting the source, a source-scoped purge, the termination purge and
//!   the account delete each remove the rows (the uuid owner columns meet the
//!   purge statements on the PostgreSQL lane as well).

use pierre_core::constants::oauth_providers::NoticeKind;
use std::sync::Arc;

use dravr_barrage::{
    ClinicalResource, Derivation, GlucoseReading, ProjectionSink, RetractReason, ReviewState,
    Scope, SelfReport, SelfReportKind, SensitivityClass, SourceKey, Tier, WellnessRecords,
};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth::providers::{SENSITIVE_T2, SENSITIVE_T3};
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use pierre_database::repositories::{
    AccountDeletionPlan, ClinicalSelection, DeletionActor, PlannedBackend, PurgeReason,
    RecordWindow,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::archive_import::sink::CachedActivitiesSink;
use pierre_providers::ai_scope;
use pierre_services::health_records::read::{
    HealthRecordsPage, RecordKind, RecordView, RecordsRead,
};
use pierre_services::provider_notice::{grant_consent, under_ai_consent};
use pierre_services::stored_health::as_stored;
use serde_json::json;
use uuid::Uuid;

use crate::archive_import_test::seed_athlete;
use crate::common;

/// The import every test projects.
const SOURCE: SourceKey = SourceKey::AppleHealthArchive;

/// A free-text note the resource carries and its stripped copy does not.
const NOTE: &str = "patient said they felt dizzy";

fn observation(id: &str) -> ClinicalResource {
    clinical(id, "Observation", Tier::T2, None)
}

fn condition(id: &str) -> ClinicalResource {
    clinical(id, "Condition", Tier::T3, Some(SensitivityClass::Mental))
}

fn clinical(
    id: &str,
    resource_type: &str,
    tier: Tier,
    sensitivity_class: Option<SensitivityClass>,
) -> ClinicalResource {
    let effective = (Timestamp::now() - Elapsed::days(3)).to_rfc3339();
    ClinicalResource {
        id: id.to_owned(),
        resource_type: resource_type.to_owned(),
        tier,
        sensitivity_class,
        fhir_version: "4.0.1".to_owned(),
        resource: json!({
            "resourceType": resource_type,
            "effectiveDateTime": effective,
            "note": [{ "text": NOTE }],
        }),
        stripped: json!({
            "resourceType": resource_type,
            "effectiveDateTime": effective,
            "subject": { "reference": "Patient/owner" },
        }),
        provenance: json!({ "resourceType": "Provenance" }),
        flat: None,
        derivation: Derivation::Verbatim,
        review_state: ReviewState::None,
        checks: Vec::new(),
        locator: None,
        read_by: None,
        legacy_id: None,
    }
}

/// An export holding every sensitive kind: two glucose readings (one in
/// mmol/L), one in a unit never stored, a self-report, an Observation (T2)
/// and a Condition raised to T3.
fn sensitive_export() -> WellnessRecords {
    let at = Timestamp::now() - Elapsed::days(2);
    let reading = |id: &str, value: f64, unit: &str, hours: i64| GlucoseReading {
        id: id.to_owned(),
        at: at + Elapsed::hours(hours),
        value,
        unit: unit.to_owned(),
        loinc: "2339-0".to_owned(),
    };
    WellnessRecords {
        glucose: vec![
            reading("g-1", 5.5, "mmol/L", 0),
            reading("g-2", 101.0, "mg/dL", 1),
            reading("g-3", 1.0, "g/L", 2),
        ],
        self_reports: vec![SelfReport {
            id: "s-1".to_owned(),
            kind: SelfReportKind::MenstrualFlow,
            start: at,
            end: at,
            value_code: Some("light".to_owned()),
            value: None,
            unit: None,
            cycle_start: true,
            protection_used: None,
            legacy_id: None,
        }],
        clinical: vec![observation("o-1"), condition("c-1")],
        ..WellnessRecords::default()
    }
}

struct Imported {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
}

impl Imported {
    fn repos(&self) -> &RepositoryRegistry {
        &self.resources.common.repos
    }

    fn scope(&self) -> Scope {
        Scope::new(self.tenant.as_uuid(), self.user_id)
    }

    fn reader(&self, view: RecordView) -> RecordsRead<'_> {
        RecordsRead {
            repos: self.repos(),
            terms: self.resources.fitness.provider_registry.as_ref(),
            user_id: self.user_id,
            tenant: &self.tenant,
            view,
            coach: None,
            project: as_stored,
        }
    }

    /// The athlete's own read of `kind`.
    async fn own(&self, kind: RecordKind) -> HealthRecordsPage {
        self.reader(RecordView::Owner)
            .read(kind, window())
            .await
            .unwrap()
    }

    /// Dravr's agent's read of `kind`, under the athlete's consents, and how
    /// many records the tier rule left out.
    async fn agents(&self, kind: RecordKind) -> (HealthRecordsPage, usize) {
        let reader = self.reader(RecordView::Served);
        let (page, withheld) = ai_scope::serve_over(
            Transport::WebApp,
            under_ai_consent(
                self.repos(),
                self.tenant.as_uuid(),
                self.user_id,
                ai_scope::ai_read(async { reader.read(kind, window()).await.unwrap() }),
            ),
        )
        .await;
        (page, withheld.sensitive_reduced)
    }

    async fn allow(&self, class: &str, name: &str) {
        grant_consent(
            self.repos(),
            self.tenant.as_uuid(),
            self.user_id,
            class,
            NoticeKind::AiConsent,
            name,
        )
        .await
        .unwrap();
    }

    /// How many self-report and clinical rows the athlete holds.
    async fn held(&self) -> (usize, usize) {
        let repos = self.repos();
        let reports = repos
            .health_records
            .list_self_reports(&self.tenant, self.user_id, window())
            .await
            .unwrap();
        let clinical = repos
            .health_records
            .list_clinical_records(
                &self.tenant,
                self.user_id,
                ClinicalSelection::Served { observations: None },
                window(),
            )
            .await
            .unwrap();
        (reports.len(), clinical.len())
    }
}

fn window() -> RecordWindow {
    RecordWindow {
        since: None,
        until: None,
        limit: 200,
    }
}

/// An athlete who imported [`sensitive_export`] twice, with no consent given.
async fn imported(label: &str) -> Imported {
    let resources = common::create_test_server_resources()
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, label).await;
    let imported = Imported {
        user_id: athlete.user_id,
        tenant: athlete.tenant,
        resources,
    };
    let sink = CachedActivitiesSink::new(Arc::clone(&imported.resources));
    // One export imported twice: a re-import replaces its own rows.
    let export = sensitive_export();
    for _ in 0..2 {
        sink.project_wellness(&imported.scope(), SOURCE, &export)
            .await
            .unwrap();
    }
    imported
}

#[tokio::test]
async fn an_import_stores_every_sensitive_record_for_the_athlete_with_no_consent() {
    let f = imported("health-records-stored").await;

    assert_eq!(f.held().await, (1, 2), "bug: a re-import doubled the rows");

    let labs = f.own(RecordKind::Labs).await;
    assert_eq!(labs.records.len(), 1, "bug: the Observation: {labs:?}");
    assert_eq!(
        labs.records[0].resource["note"][0]["text"], NOTE,
        "bug: the athlete reads the resource as read"
    );
    let clinical = f.own(RecordKind::Clinical).await;
    assert_eq!(
        clinical.records.len(),
        1,
        "bug: the Condition: {clinical:?}"
    );
    assert_eq!(clinical.records[0].tier, "T3", "bug: its tier");
    let reports = f.own(RecordKind::SelfReports).await;
    assert_eq!(reports.self_reports.len(), 1, "bug: the self-report");
    assert_eq!(
        reports.self_reports[0].kind, "menstrual_flow",
        "bug: its kind"
    );

    let glucose = f.own(RecordKind::Glucose).await;
    let mut values: Vec<f64> = glucose.readings.iter().map(|r| r.mg_dl).collect();
    values.sort_by(f64::total_cmp);
    assert_eq!(
        values.len(),
        2,
        "bug: the unconverted unit was stored: {values:?}"
    );
    assert!(
        (values[0] - 99.0858).abs() < 1e-6,
        "bug: 5.5 mmol/L read as {}",
        values[0]
    );
    assert!(
        (values[1] - 101.0).abs() < 1e-9,
        "bug: the mg/dL reading moved"
    );
}

#[tokio::test]
async fn dravrs_agent_reads_each_tier_only_under_its_consent_and_never_the_resource() {
    let f = imported("health-records-agent").await;

    for kind in [
        RecordKind::Labs,
        RecordKind::Clinical,
        RecordKind::SelfReports,
        RecordKind::Glucose,
    ] {
        let (page, withheld) = f.agents(kind).await;
        assert_eq!(
            page.records.len() + page.self_reports.len() + page.readings.len(),
            0,
            "bug: {kind:?} read with no consent: {page:?}"
        );
        assert!(
            withheld > 0,
            "bug: {kind:?}'s withheld records were not tallied"
        );
    }

    f.allow(SENSITIVE_T2, "Health records").await;
    let (labs, _) = f.agents(RecordKind::Labs).await;
    assert_eq!(labs.records.len(), 1, "bug: T2 under sensitive_t2");
    assert!(
        labs.records[0].resource.get("note").is_none(),
        "bug: the agent read the free text: {:?}",
        labs.records[0].resource
    );
    assert_eq!(
        labs.records[0].resource["subject"]["reference"], "Patient/owner",
        "bug: not the stripped copy"
    );
    let (glucose, _) = f.agents(RecordKind::Glucose).await;
    assert_eq!(glucose.readings.len(), 2, "bug: glucose under sensitive_t2");
    let (clinical, withheld) = f.agents(RecordKind::Clinical).await;
    assert!(
        clinical.records.is_empty(),
        "bug: a T3 record under sensitive_t2 alone"
    );
    assert_eq!(withheld, 1, "bug: the T3 record was not tallied");

    f.allow(SENSITIVE_T3, "Intimate health records").await;
    let (clinical, _) = f.agents(RecordKind::Clinical).await;
    assert_eq!(clinical.records.len(), 1, "bug: T3 under sensitive_t3");
    let (reports, _) = f.agents(RecordKind::SelfReports).await;
    assert_eq!(
        reports.self_reports.len(),
        1,
        "bug: a self-report under sensitive_t3"
    );
}

#[tokio::test]
async fn retracting_the_import_deletes_its_records() {
    let f = imported("health-records-retract").await;
    CachedActivitiesSink::new(Arc::clone(&f.resources))
        .retract(&f.scope(), SOURCE, None, RetractReason::SourceDeleted)
        .await
        .unwrap();
    assert_eq!(
        f.held().await,
        (0, 0),
        "bug: the records outlived their source"
    );
    assert!(
        f.own(RecordKind::Glucose).await.readings.is_empty(),
        "bug: the glucose outlived its source"
    );
}

#[tokio::test]
async fn the_termination_purge_deletes_the_providers_records() {
    let f = imported("health-records-terminate").await;
    let purge = f
        .repos()
        .provider_data
        .purge_provider_data(SOURCE.as_str())
        .await
        .unwrap();
    assert_eq!(purge.removed_from("self_reports"), 1, "bug: {purge:?}");
    assert_eq!(purge.removed_from("clinical_records"), 2, "bug: {purge:?}");
    assert_eq!(
        f.held().await,
        (0, 0),
        "bug: the records outlived the purge"
    );
}

#[tokio::test]
async fn the_users_provider_purge_counts_the_records() {
    let f = imported("health-records-user-purge").await;
    let purge = f
        .repos()
        .provider_data
        .purge_user_provider_data(
            f.user_id,
            &f.tenant,
            SOURCE.as_str(),
            PurgeReason::AthleteDeletedImport,
        )
        .await
        .unwrap();
    assert_eq!(purge.removed_from("self_reports"), 1, "bug: {purge:?}");
    assert_eq!(purge.removed_from("clinical_records"), 2, "bug: {purge:?}");
}

#[tokio::test]
async fn deleting_the_account_deletes_the_records() {
    let f = imported("health-records-account").await;
    f.repos()
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id: f.user_id,
            actor: DeletionActor::Admin,
            firebase_project: None,
            backends: vec![PlannedBackend {
                tenant_id: f.tenant,
                backend: SOURCE.as_str().to_owned(),
            }],
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        f.held().await,
        (0, 0),
        "bug: the records outlived the account"
    );
}
