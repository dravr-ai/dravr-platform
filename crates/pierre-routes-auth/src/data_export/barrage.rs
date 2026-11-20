// ABOUTME: Barrage's part of the athlete's export: their activity records, erasures and record_log, in every tenant they belong to
// ABOUTME: Written as three sections of the export's tables, just before cached_activities, a row at a time

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_barrage::export::{
    self as barrage_export, ERASED_TABLE, RECORDS_TABLE, RECORD_LOG_TABLE,
};
use dravr_barrage::record_log::checkpoint::LogProof;
use dravr_barrage::record_log::entry::LogEntry;
use dravr_barrage::record_log::{all_entries, athlete_proof, RecordLogKey};
use dravr_barrage::store::{ActivityRecordStore, ErasedActivity};
use dravr_barrage::{BarrageError, Scope};
use futures_util::stream::{self, BoxStream, StreamExt};
use futures_util::TryStreamExt;
use pierre_core::models::TenantId;
use pierre_database::repositories::{UserExportRepository, UserExportTable, USER_EXPORT_TABLES};
use pierre_services::barrage_keys::CheckpointKey;
use serde::Serialize;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::{ChunkSink, ExportSummary, Writer};

/// The platform table barrage's sections come just before: the projection
/// they feed.
pub(super) const BARRAGE_BEFORE: &str = "cached_activities";

/// What a document is read from.
pub(super) struct ExportSources<'src> {
    /// The platform's tables.
    pub(super) repository: &'src dyn UserExportRepository,
    /// Barrage's records and erasures: the context's one record store.
    pub(super) records: &'src dyn ActivityRecordStore,
    /// The tenants the athlete's rows reach.
    pub(super) tenants: &'src [TenantId],
    /// The latest originals bundle's view, or `null`.
    pub(super) originals: Value,
    /// The key this deployment signs `record_log` checkpoints with; `None`
    /// while it signs none, and the log's proof is then `null`.
    pub(super) checkpoint_key: Option<&'src CheckpointKey>,
}

/// One section of `tables`, in the order the document writes them.
#[derive(Clone, Copy)]
pub(super) enum Section {
    /// A platform table.
    Platform(&'static UserExportTable),
    /// `barrage.activity_records`.
    Records,
    /// `barrage.erased_activities`.
    Erased,
    /// `barrage.record_log`: the athlete's log, how each record pairs with
    /// it, and the signed checkpoint's proof.
    Log,
}

impl Section {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Platform(table) => table.name,
            Self::Records => RECORDS_TABLE,
            Self::Erased => ERASED_TABLE,
            Self::Log => RECORD_LOG_TABLE,
        }
    }

    /// Every section: the platform's tables, barrage's three just before
    /// [`BARRAGE_BEFORE`].
    pub(super) fn all() -> Vec<Self> {
        let mut sections = Vec::with_capacity(USER_EXPORT_TABLES.len() + 3);
        for table in USER_EXPORT_TABLES {
            if table.name == BARRAGE_BEFORE {
                sections.extend([Self::Records, Self::Erased, Self::Log]);
            }
            sections.push(Self::Platform(table));
        }
        sections
    }
}

/// The barrage scope of `user_id` in `tenant`.
pub(super) fn scope_of(tenant: TenantId, user_id: Uuid) -> Scope {
    Scope::new(Uuid::from(tenant), user_id)
}

/// An erasure as the document writes it: with the tenant it was made in.
#[derive(Serialize)]
struct ErasedRow {
    tenant_id: Uuid,
    #[serde(flatten)]
    erased: ErasedActivity,
}

/// One tenant's log as the document writes it.
///
/// Every entry, the key each record's entries carry (the record is in
/// `barrage.activity_records`), and the proof against the newest signed
/// checkpoint holding the athlete (`null` before any, or while this
/// deployment signs none). A verifier rebuilds the athlete's tree from
/// `entries`, checks the leaf's path to the note's root and the note's
/// Ed25519 signature (`dravr_barrage::record_log::checkpoint::verify_export`,
/// or any C2SP signed-note verifier for the note).
#[derive(Serialize)]
struct LogRow {
    tenant_id: Uuid,
    entries: Vec<LogEntry>,
    log_keys: Vec<RecordLogKey>,
    proof: Option<LogProof>,
}

/// `scope`'s log row.
async fn log_row(sources: &ExportSources<'_>, scope: Scope) -> Result<LogRow, BarrageError> {
    let log = sources.records;
    let entries = all_entries(log, &scope).await?;
    let log_keys = barrage_export::export_log_keys(log, scope)
        .try_collect()
        .await?;
    let proof = match sources.checkpoint_key {
        Some(key) => athlete_proof(log, &scope, &key.name, &key.public_key).await?,
        None => None,
    };
    Ok(LogRow {
        tenant_id: scope.tenant_id.0,
        entries,
        log_keys,
        proof,
    })
}

/// Write barrage's `section` as `"name":[row,…]`, each tenant's rows in turn,
/// a row at a time, each in barrage's own public shape.
pub(super) async fn write_barrage<S: ChunkSink>(
    section: Section,
    sources: &ExportSources<'_>,
    user_id: Uuid,
    writer: &mut Writer<'_, S>,
    summary: &mut ExportSummary,
) {
    let name = section.name();
    writer.put(format!("\"{name}\":[").as_bytes()).await;
    let mut first = true;
    for tenant in sources.tenants {
        let scope = scope_of(*tenant, user_id);
        let more = if matches!(section, Section::Log) {
            let row = stream::once(log_row(sources, scope));
            write_rows(name, row.boxed(), &mut first, writer, summary).await
        } else if matches!(section, Section::Erased) {
            let rows = barrage_export::export_erased(sources.records, scope);
            let rows = rows.map_ok(move |erased| ErasedRow {
                tenant_id: scope.tenant_id.0,
                erased,
            });
            write_rows(name, rows.boxed(), &mut first, writer, summary).await
        } else {
            let rows = barrage_export::export_user(sources.records, scope);
            write_rows(name, rows, &mut first, writer, summary).await
        };
        if !more {
            return;
        }
    }
    writer.put(b"]").await;
}

/// Write `rows` into the open array `name`; `false` when the file ran out of
/// room, after closing the array and naming `name` among the skipped.
async fn write_rows<S: ChunkSink, T: Serialize>(
    name: &'static str,
    mut rows: BoxStream<'_, Result<T, BarrageError>>,
    first: &mut bool,
    writer: &mut Writer<'_, S>,
    summary: &mut ExportSummary,
) -> bool {
    while !writer.disconnected {
        let row = match rows.try_next().await {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(error) => {
                warn!(table = name, error = %error, "User export could not read a table");
                summary.mark_failed(name);
                break;
            }
        };
        if let Some(limit) = writer.out_of_room() {
            summary.limit_reached = Some(limit);
            summary.skipped_tables.push(name);
            writer.put(b"]").await;
            return false;
        }
        if !*first {
            writer.put(b",").await;
        }
        *first = false;
        match serde_json::to_vec(&row) {
            Ok(text) => writer.put(&text).await,
            Err(error) => {
                warn!(table = name, error = %error, "User export could not write a row");
                summary.mark_failed(name);
                writer.put(b"null").await;
            }
        }
        summary.rows += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use dravr_barrage::export::EXPORT_WITHHELD;
    use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
    use dravr_barrage::store::memory::MemoryActivityRecordStore;
    use dravr_barrage::{ActivityId, ActivityRecord, Provenance, SourceKey};
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_services::barrage_keys::memory_records;
    use serde_json::json;

    use super::super::tests::{collector, document, sources, Collector, FakeRepository, ROOMY};
    use super::super::{write_export, ExportLimits};
    use super::*;

    /// `count` Strava archive records of `user_id` in `tenant`.
    async fn barrage_history(
        records: &MemoryActivityRecordStore,
        tenant: TenantId,
        user_id: Uuid,
        count: usize,
    ) {
        let scope = scope_of(tenant, user_id);
        let rows: Vec<ActivityRecord> = (0..count)
            .map(|i| {
                let id = format!("ride-{i:04}");
                let start = Utc.with_ymd_and_hms(2026, 3, 1, 8, 0, 0).unwrap();
                let activity = ActivityBuilder::new(
                    &id,
                    "Ride",
                    SportType::Ride,
                    start,
                    3600,
                    "strava_archive",
                )
                .distance_meters(30_000.0)
                .build();
                let provenance = Provenance {
                    source: SourceKey::StravaArchive,
                    import_id: None,
                    source_activity_id: id.clone(),
                    source_athlete_id: None,
                    ingested_at: start,
                    original: None,
                    original_sha256: None,
                    session_index: None,
                    streams_state: StreamsState::None,
                    source_gear: None,
                    importer_version: None,
                    summary_window: SummaryWindow::default(),
                    totals_from: TotalsFrom::default(),
                };
                ActivityRecord::try_new(
                    scope,
                    ActivityId::parse(&id).unwrap(),
                    provenance,
                    activity,
                )
                .unwrap()
            })
            .collect();
        records.upsert(&scope, &rows).await.unwrap();
    }

    /// Where `key` first appears in the written bytes.
    fn position(sink: &Collector, key: &str) -> usize {
        let bytes: Vec<u8> = sink.chunks.iter().flat_map(|c| c.to_vec()).collect();
        let text = String::from_utf8(bytes).unwrap();
        text.find(&format!("\"{key}\":"))
            .unwrap_or_else(|| panic!("{key} is in the document"))
    }

    /// The log section carries every entry, the record-to-entry keys, and a
    /// proof that verifies against the signed checkpoint, as an athlete's
    /// verifier would check it.
    #[tokio::test]
    async fn the_exported_log_verifies_against_its_signed_checkpoint() {
        use dravr_barrage::digest::Sha256;
        use dravr_barrage::keyed::Signer;
        use dravr_barrage::record_log::checkpoint::{verify_export, Anchor};
        use dravr_barrage::record_log::cut_checkpoint;
        use pierre_services::barrage_keys::{CheckpointKey, LocalSigner};
        use ring::signature::{UnparsedPublicKey, ED25519};

        let repository = FakeRepository {
            rows_per_table: 0,
            row_bytes: 4,
            failing: None,
        };
        let records = memory_records();
        let tenant = TenantId::from_uuid(Uuid::new_v4());
        let user_id = Uuid::new_v4();
        barrage_history(&records, tenant, user_id, 3).await;
        let signer = LocalSigner::random().unwrap();
        let anchor = Anchor {
            seq: 2,
            hash: Sha256::of(b"receipt 2"),
        };
        cut_checkpoint(&records, &signer, anchor, "test", Utc::now())
            .await
            .unwrap()
            .expect("a checkpoint is cut");
        let key = CheckpointKey {
            name: signer.key_name().to_owned(),
            public_key: signer.public_key(),
        };
        let mut sink = collector();
        write_export(
            ExportSources {
                repository: &repository,
                records: &records,
                tenants: &[tenant],
                originals: Value::Null,
                checkpoint_key: Some(&key),
            },
            user_id,
            &mut sink,
            ROOMY,
        )
        .await;

        let doc = document(&sink);
        let rows = doc["tables"][RECORD_LOG_TABLE].as_array().unwrap();
        assert_eq!(rows.len(), 1, "bug: one log row per tenant");
        let entries: Vec<LogEntry> = serde_json::from_value(rows[0]["entries"].clone()).unwrap();
        let proof: LogProof = serde_json::from_value(rows[0]["proof"].clone()).unwrap();
        assert_eq!(entries.len(), 3, "bug: every put is exported");
        assert_eq!(
            rows[0]["log_keys"].as_array().map(Vec::len),
            Some(3),
            "bug: a record cannot be paired with its entries"
        );
        let verify = |key: &[u8; 32], message: &[u8], sig: &[u8; 64]| {
            UnparsedPublicKey::new(&ED25519, key)
                .verify(message, sig)
                .is_ok()
        };
        let verified = verify_export(&entries, &proof, &verify).unwrap();
        assert_eq!(
            verified.anchored, 3,
            "bug: the checkpoint does not cover the log"
        );

        let mut forged = entries;
        forged[1].content = Some(Sha256::of(b"rewritten"));
        assert!(
            verify_export(&forged, &proof, &verify).is_err(),
            "bug: a rewritten entry still verifies"
        );
    }

    #[tokio::test]
    async fn barrages_sections_sit_just_before_the_projection_they_feed() {
        let repository = FakeRepository {
            rows_per_table: 1,
            row_bytes: 4,
            failing: None,
        };
        let records = memory_records();
        let tenant = TenantId::from_uuid(Uuid::new_v4());
        let user_id = Uuid::new_v4();
        barrage_history(&records, tenant, user_id, 3).await;
        let mut sink = collector();
        let summary = write_export(
            ExportSources {
                repository: &repository,
                records: &records,
                tenants: &[tenant],
                originals: json!({ "bundle_id": "b" }),
                checkpoint_key: None,
            },
            user_id,
            &mut sink,
            ROOMY,
        )
        .await;

        let doc = document(&sink);
        assert_eq!(doc["complete"], true, "bug: {:?}", summary.failed_tables);
        assert_eq!(
            doc["tables"][RECORDS_TABLE].as_array().unwrap().len(),
            3,
            "bug: every barrage record of the athlete is in the export"
        );
        assert_eq!(
            doc["tables"][RECORDS_TABLE][0]["provenance"]["source"], "strava_archive",
            "bug: a record is written in barrage's own shape"
        );
        let before = USER_EXPORT_TABLES
            .iter()
            .position(|table| table.name == BARRAGE_BEFORE)
            .unwrap();
        let previous = USER_EXPORT_TABLES[before - 1].name;
        assert!(
            position(&sink, previous) < position(&sink, RECORDS_TABLE)
                && position(&sink, RECORDS_TABLE) < position(&sink, ERASED_TABLE)
                && position(&sink, ERASED_TABLE) < position(&sink, BARRAGE_BEFORE),
            "bug: barrage's sections sit immediately before {BARRAGE_BEFORE}"
        );
        assert!(
            position(&sink, "originals") < position(&sink, "tables"),
            "bug: the originals bundle is named in the head, before the tables"
        );
        assert_eq!(
            doc["originals"]["bundle_id"], "b",
            "bug: the head carries the originals view it was given"
        );
        let withheld: Vec<&str> = doc["withheld"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["table"].as_str().unwrap())
            .collect();
        for (table, _) in EXPORT_WITHHELD {
            assert!(withheld.contains(table), "bug: {table} is named withheld");
        }
    }

    #[tokio::test]
    async fn a_size_limit_inside_barrages_records_names_them_skipped() {
        let repository = FakeRepository {
            rows_per_table: 1,
            row_bytes: 4,
            failing: None,
        };
        let records = memory_records();
        let tenant = TenantId::from_uuid(Uuid::new_v4());
        let user_id = Uuid::new_v4();
        barrage_history(&records, tenant, user_id, 200).await;
        // Room for the platform tables before barrage's, not for 200 records.
        let mut probe = collector();
        write_export(sources(&repository), user_id, &mut probe, ROOMY).await;
        let before_barrage = position(&probe, BARRAGE_BEFORE);
        let limits = ExportLimits {
            max_bytes: before_barrage + 4_000,
            ..ROOMY
        };
        let mut sink = collector();
        write_export(
            ExportSources {
                repository: &repository,
                records: &records,
                tenants: &[tenant],
                originals: Value::Null,
                checkpoint_key: None,
            },
            user_id,
            &mut sink,
            limits,
        )
        .await;
        let doc = document(&sink);
        assert_eq!(
            doc["complete"], false,
            "bug: a document cut by its size limit says it is incomplete"
        );
        assert_eq!(
            doc["limit_reached"], "size",
            "bug: the document names the size limit that cut it"
        );
        let skipped: Vec<&str> = doc["skipped_tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        assert_eq!(
            skipped.first(),
            Some(&RECORDS_TABLE),
            "bug: the barrage section the limit cut is the first one skipped: {skipped:?}"
        );
        assert!(
            skipped.contains(&ERASED_TABLE) && skipped.contains(&BARRAGE_BEFORE),
            "bug: every section after the cut is skipped too: {skipped:?}"
        );
    }
}
