// ABOUTME: Provider-terms filtering of stored health records — sleep, recovery and body metrics as persisted
// ABOUTME: Each record is governed by its data source's provider and upstream source, before any cross-source merge

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a model, or a caller over an external transport, may see of the
//! athlete's stored health records (carnet#771).
//!
//! A stored record names the data source that synced it, never a provider,
//! so the filter resolves each record's provider and upstream source through
//! the athlete's [`DataSource`] rows before the shared provider-terms filter
//! runs. The parent module re-exports the public surface.

use std::collections::HashMap;

use pierre_core::ai_policy::ProviderTerms;
use pierre_core::models::{
    DataSource, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::filter_typed;

/// A stored health record — a night's sleep, a day's recovery, a day's body
/// metrics — as the sync pipeline persisted it.
///
/// The record names the data source that synced it, never a provider: the
/// provider and upstream source live on the [`DataSource`] row.
pub trait StoredHealthRecord: Serialize + DeserializeOwned {
    /// The id of the data source that synced this record.
    fn data_source_id(&self) -> &str;
    /// The provider the record was stored under.
    fn source_name(&self) -> &str;
}

impl StoredHealthRecord for StoredSleepSession {
    fn data_source_id(&self) -> &str {
        &self.data_source_id
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

impl StoredHealthRecord for StoredRecoveryMetrics {
    fn data_source_id(&self) -> &str {
        &self.data_source_id
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

impl StoredHealthRecord for StoredHealthMetrics {
    fn data_source_id(&self) -> &str {
        &self.data_source_id
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

/// The string fields every stored health record needs to deserialize, filled
/// with a neutral placeholder when a rule removed them.
const HEALTH_RECORD_REQUIRED: &[&str] = &["id", "user_id", "data_source_id", "source_name"];

/// Stored health records as a model, or a caller over an external transport,
/// may see them; unchanged when no gate applies (carnet#771).
///
/// Each record is governed by its data source's `provider` and upstream
/// `source`, looked up in `sources` — the athlete's data sources, read once
/// per call rather than once per record. A record whose data source is not
/// among them is governed by the provider it was stored under.
///
/// Filter before merging records across sources: a merge folds one source's
/// metrics into another's record, past the reach of any filter after it.
#[must_use]
pub fn filter_stored_health<T: StoredHealthRecord>(
    lookup: &dyn ProviderTerms,
    records: Vec<T>,
    sources: &[DataSource],
) -> Vec<T> {
    let by_id: HashMap<&str, &DataSource> = sources.iter().map(|s| (s.id.as_str(), s)).collect();
    filter_typed(
        lookup,
        records,
        |record| {
            by_id.get(record.data_source_id()).map_or_else(
                || (record.source_name().to_ascii_lowercase(), None),
                |source| (source.provider.to_ascii_lowercase(), source.source.clone()),
            )
        },
        HEALTH_RECORD_REQUIRED,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_scope::tests::first_party;
    use crate::ai_scope::{ai_read, serve_over, unfiltered, with_ai_consent};
    use crate::provider_terms::{NOLIO, NOLIO_TRANSPORT, WHOOP};
    use chrono::{Duration, NaiveDate, TimeZone, Utc};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::models::{merge_recovery_metrics, merge_sleep_sessions, DeviceType};
    use pierre_core::transport::{Transport, TransportPolicy};
    use std::collections::BTreeSet;

    /// Nolio first-party only with its connector rules, WHOOP scores kept
    /// from models, Garmin unrestricted.
    struct HealthTerms;

    impl ProviderTerms for HealthTerms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "nolio" => Some(&NOLIO),
                "whoop" => Some(&WHOOP),
                "garmin" => Some(&SourcePolicy::ALLOW_ALL),
                _ => None,
            }
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            match provider {
                "nolio" => Some(NOLIO_TRANSPORT),
                "whoop" | "garmin" => Some(TransportPolicy::AnyTransport),
                _ => None,
            }
        }
    }

    fn data_source(id: &str, provider: &str, source: Option<&str>) -> DataSource {
        DataSource {
            id: id.to_owned(),
            user_id: "athlete".to_owned(),
            provider: provider.to_owned(),
            device_model: None,
            software_version: None,
            source: source.map(str::to_owned),
            device_type: DeviceType::Unknown,
            original_source_name: None,
        }
    }

    /// The athlete's sources: a Garmin watch, a WHOOP strap, and a Nolio
    /// account relaying a Garmin watch and a Zepp band.
    fn sources() -> Vec<DataSource> {
        vec![
            data_source("ds-garmin", "garmin", None),
            data_source("ds-whoop", "whoop", None),
            data_source("ds-nolio-garmin", "nolio", Some("garmin")),
            data_source("ds-nolio-zepp", "nolio", Some("zepp")),
        ]
    }

    /// One night every source recorded, each with metrics of its own.
    fn night(data_source_id: &str, source_name: &str) -> StoredSleepSession {
        let start = Utc.with_ymd_and_hms(2026, 9, 29, 22, 0, 0).unwrap();
        let from = |name: &str| source_name.eq_ignore_ascii_case(name);
        StoredSleepSession {
            id: format!("sleep-{data_source_id}"),
            user_id: "athlete".to_owned(),
            data_source_id: data_source_id.to_owned(),
            is_nap: false,
            start_datetime: start,
            end_datetime: start + Duration::hours(8),
            total_sleep_seconds: Some(7 * 3600),
            deep_sleep_seconds: from("nolio").then_some(5400),
            light_sleep_seconds: None,
            rem_sleep_seconds: None,
            awake_seconds: None,
            sleep_efficiency: None,
            avg_heart_rate: None,
            min_heart_rate: from("garmin").then_some(48),
            avg_hrv: from("whoop").then_some(71.0),
            sleep_score: from("whoop").then_some(88),
            stages: Vec::new(),
            source_name: source_name.to_owned(),
        }
    }

    fn morning(data_source_id: &str, source_name: &str) -> StoredRecoveryMetrics {
        StoredRecoveryMetrics {
            id: format!("recovery-{data_source_id}"),
            user_id: "athlete".to_owned(),
            data_source_id: data_source_id.to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            recovery_score: Some(34),
            readiness_score: None,
            hrv_ms: Some(65.0),
            hrv_rmssd: None,
            resting_heart_rate: Some(if source_name == "nolio" { 39 } else { 50 }),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: None,
            source_name: source_name.to_owned(),
            recorded_at: Utc.with_ymd_and_hms(2026, 9, 30, 7, 0, 0).unwrap(),
        }
    }

    fn nights() -> Vec<StoredSleepSession> {
        vec![
            night("ds-garmin", "garmin"),
            night("ds-whoop", "whoop"),
            night("ds-nolio-garmin", "nolio"),
            night("ds-nolio-zepp", "nolio"),
        ]
    }

    fn data_source_ids<T: StoredHealthRecord>(records: &[T]) -> Vec<&str> {
        records
            .iter()
            .map(StoredHealthRecord::data_source_id)
            .collect()
    }

    #[tokio::test]
    async fn an_external_call_gets_no_first_party_only_health_record_and_the_rest_still_merge() {
        let (kept, withheld) = serve_over(
            Transport::McpHttp,
            // An athlete who consented to AI use of WHOOP data (carnet#726).
            with_ai_consent(
                BTreeSet::new(),
                ai_read(async { filter_stored_health(&HealthTerms, nights(), &sources()) }),
            ),
        )
        .await;
        assert_eq!(data_source_ids(&kept), vec!["ds-garmin", "ds-whoop"]);
        assert_eq!(
            withheld.off_interface, 2,
            "both Nolio rows stay first-party"
        );

        let merged = merge_sleep_sessions(kept);
        assert_eq!(merged.len(), 1, "the two permitted sources are one night");
        let night = &merged[0];
        assert_eq!(night.sources.len(), 2);
        assert!(
            night.sources.iter().all(|s| s != "nolio"),
            "{:?}",
            night.sources
        );
        assert_eq!(
            night.record.deep_sleep_seconds, None,
            "no Nolio metric fills the served night"
        );
        assert_eq!(night.record.min_heart_rate, Some(48));
        assert_eq!(night.record.avg_hrv, Some(71.0));
        assert_eq!(
            night.record.sleep_score, None,
            "WHOOP's own score never reaches the model"
        );
    }

    #[tokio::test]
    async fn a_model_read_applies_each_sources_ai_rules_before_the_merge() {
        let mornings = vec![
            morning("ds-garmin", "garmin"),
            morning("ds-whoop", "whoop"),
            morning("ds-nolio-zepp", "nolio"),
        ];
        let (kept, withheld) = first_party(ai_read(async {
            filter_stored_health(&HealthTerms, mornings, &sources())
        }))
        .await;
        assert_eq!(
            data_source_ids(&kept),
            vec!["ds-garmin", "ds-whoop"],
            "a relayed Zepp reading is denied to AI"
        );
        assert_eq!(kept[0].recovery_score, Some(34), "Garmin's score stays");
        assert_eq!(kept[1].recovery_score, None, "WHOOP's score is redacted");
        assert_eq!(kept[1].hrv_ms, Some(65.0), "WHOOP's measurements stay");
        assert_eq!((withheld.dropped, withheld.reduced), (1, 1));
        assert_eq!(withheld.off_interface, 0);

        let merged = merge_recovery_metrics(kept);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].record.resting_heart_rate,
            Some(50),
            "the denied Zepp reading fills nothing"
        );
    }

    #[tokio::test]
    async fn outside_every_gate_the_athlete_keeps_every_health_record() {
        let route = filter_stored_health(&HealthTerms, nights(), &sources());
        assert_eq!(route.len(), 4);
        assert_eq!(route[1].sleep_score, Some(88));

        let (cache, withheld) = serve_over(
            Transport::McpHttp,
            ai_read(unfiltered(async {
                filter_stored_health(&HealthTerms, nights(), &sources())
            })),
        )
        .await;
        assert_eq!(cache.len(), 4);
        assert!(withheld.is_empty());
    }

    #[tokio::test]
    async fn a_record_without_its_data_source_is_governed_by_the_provider_it_was_stored_under() {
        let orphan = night("ds-gone", "Nolio");
        let (kept, withheld) = serve_over(
            Transport::A2a,
            ai_read(async { filter_stored_health(&HealthTerms, vec![orphan], &sources()) }),
        )
        .await;
        assert!(kept.is_empty());
        assert_eq!(withheld.off_interface, 1);
    }
}
