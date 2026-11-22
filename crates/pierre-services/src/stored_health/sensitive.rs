// ABOUTME: What of a stored wellness row the tier gate keeps from the reader: clinical readings and the athlete's daily note
// ABOUTME: Glucose (T2) under the tier rule; the daily note (T3) off an outside client without sensitive_t3 (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The sensitive fields of the stored rows a merged read serves, nulled
//! before the merge where the reader may not have them, each nulled field
//! tallied once by the tier gate ([`ai_scope::withhold_tier`]).

use dravr_barrage::Tier;
use pierre_core::models::{StoredHealthMetrics, StoredRecoveryMetrics};
use pierre_providers::ai_scope;

/// Blood pressure and glucose are clinical readings, whichever source wrote
/// them: an external transport reads the rest of a body record without its
/// blood pressure, and self-measured glucose, a T2 record, is read only
/// where the tier gate lets it ([`ai_scope::tier_readable`]): never over an
/// external transport, and by a model or a human coach only once the
/// athlete allowed their health records' AI use, each nulled reading tallied
/// once. One gate, no second filter.
pub(super) fn withhold_clinical_readings(rows: &mut [StoredHealthMetrics]) {
    if ai_scope::serving_external() {
        for row in rows.iter_mut() {
            row.systolic_bp = None;
            row.diastolic_bp = None;
        }
    }
    if ai_scope::tier_readable(Tier::T2) {
        return;
    }
    for row in rows.iter_mut().filter(|row| row.blood_glucose.is_some()) {
        row.blood_glucose = None;
        ai_scope::withhold_tier(Tier::T2);
    }
}

/// The note the athlete typed into their source's daily log is theirs, a T3
/// record: an outside client (MCP, A2A, an API key) reads it only while the
/// athlete gives `sensitive_t3`, by the tier rule
/// ([`ai_scope::tier_readable`]), each nulled note tallied once as the
/// athlete's choice. Dravr's own agent and the human coach read it as before
/// (E3; JF 2026-11-20, "Gate for third parties").
pub(super) fn withhold_daily_notes(rows: &mut [StoredRecoveryMetrics]) {
    if !ai_scope::serving_external() || ai_scope::tier_readable(Tier::T3) {
        return;
    }
    for row in rows.iter_mut().filter(|row| row.athlete_note.is_some()) {
        row.athlete_note = None;
        ai_scope::withhold_tier(Tier::T3);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use dravr_tronc::dates::Timestamp;
    use pierre_core::constants::oauth_providers::SENSITIVE_T3;
    use pierre_core::transport::Transport;
    use pierre_providers::ai_scope::{ai_read, serve_over, with_ai_consent};

    use super::*;

    fn morning(note: Option<&str>) -> StoredRecoveryMetrics {
        StoredRecoveryMetrics {
            id: "day".to_owned(),
            user_id: "athlete".to_owned(),
            data_source_id: "ds".to_owned(),
            date: Timestamp::now().utc_date(),
            recovery_score: None,
            readiness_score: Some(70),
            hrv_ms: None,
            hrv_rmssd: Some(64.0),
            resting_heart_rate: Some(47),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: note.map(str::to_owned),
            source_name: "intervals_icu".to_owned(),
            recorded_at: Timestamp::now(),
        }
    }

    /// The notes `rows` keep read over `transport` under consents
    /// `withheld`, and the sensitive tally.
    async fn read(transport: Transport, withheld: &[&str]) -> (Vec<Option<String>>, usize) {
        let withheld: BTreeSet<String> = withheld.iter().map(|key| (*key).to_owned()).collect();
        let (notes, tally) = serve_over(
            transport,
            with_ai_consent(
                withheld,
                ai_read(async {
                    let mut rows = vec![morning(Some("legs heavy")), morning(None)];
                    withhold_daily_notes(&mut rows);
                    rows.into_iter().map(|row| row.athlete_note).collect()
                }),
            ),
        )
        .await;
        (notes, tally.sensitive_reduced)
    }

    #[tokio::test]
    async fn an_outside_client_reads_the_daily_note_only_under_sensitive_t3() {
        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            assert_eq!(
                read(transport, &[SENSITIVE_T3]).await,
                (vec![None, None], 1),
                "bug: withheld and tallied once per note ({transport:?})"
            );
            assert_eq!(
                read(transport, &[]).await,
                (vec![Some("legs heavy".to_owned()), None], 0),
                "given: the note is served ({transport:?})"
            );
        }
    }

    #[tokio::test]
    async fn dravrs_own_agent_reads_the_daily_note_without_consent() {
        assert_eq!(
            read(Transport::WebApp, &[SENSITIVE_T3]).await,
            (vec![Some("legs heavy".to_owned()), None], 0),
            "bug: E3 keeps the note for the first-party agent"
        );
    }
}
