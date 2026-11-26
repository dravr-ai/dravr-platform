// ABOUTME: What of a stored wellness row the tier gate keeps from the reader: clinical readings and the athlete's daily note
// ABOUTME: Glucose (T2) under the tier rule; the daily note (T3) never to an outside client, consent or not (carnet#1544)

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
/// record. An outside client (MCP, A2A, an API key) never reads it,
/// `sensitive_t3` given or not (JF 2026-11-25, carnet#1544 slice 4b), and
/// Dravr's own agent, theirs or their coach's, reads it only while the athlete
/// gives `sensitive_t3` (JF 2026-11-25, "Gate on tier consent"): both by the
/// tier rule ([`ai_scope::ai_reads_tier`]), each nulled note tallied once. A
/// human coach's own screen reads what the athlete shares with them, no AI
/// asked.
pub(super) fn withhold_daily_notes(rows: &mut [StoredRecoveryMetrics]) {
    if ai_scope::ai_reads_tier(Tier::T3) {
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
    use pierre_providers::ai_scope::{ai_read, for_human_coach, serve_over, with_ai_consent};

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
        // The notes, and how many the interface's ceiling withheld.
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
        (notes, tally.off_interface)
    }

    #[tokio::test]
    async fn an_outside_client_never_reads_the_daily_note_even_under_sensitive_t3() {
        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            for withheld in [&[SENSITIVE_T3][..], &[][..]] {
                assert_eq!(
                    read(transport, withheld).await,
                    (vec![None, None], 1),
                    "bug: withheld and tallied once per note ({transport:?}, {withheld:?})"
                );
            }
        }
    }

    #[tokio::test]
    async fn dravrs_own_agent_reads_the_daily_note_only_under_sensitive_t3() {
        for transport in [
            Transport::WebApp,
            Transport::MobileApp,
            Transport::Messaging,
        ] {
            assert_eq!(
                read(transport, &[SENSITIVE_T3]).await,
                (vec![None, None], 1),
                "bug: JF 2026-11-25 holds the agent to the tier consent ({transport:?})"
            );
            assert_eq!(
                read(transport, &[]).await,
                (vec![Some("legs heavy".to_owned()), None], 0),
                "given: the agent reads it ({transport:?})"
            );
        }
    }

    #[tokio::test]
    async fn the_athletes_and_the_coachs_own_screens_keep_the_note() {
        let withheld = BTreeSet::from([SENSITIVE_T3.to_owned()]);
        let rows = |withheld: BTreeSet<String>, coach: bool| async move {
            let read = async {
                let mut rows = vec![morning(Some("legs heavy"))];
                withhold_daily_notes(&mut rows);
                rows.remove(0).athlete_note
            };
            let read = with_ai_consent(withheld, read);
            if coach {
                serve_over(Transport::WebApp, for_human_coach(read)).await
            } else {
                serve_over(Transport::WebApp, read).await
            }
        };
        assert_eq!(
            rows(withheld.clone(), false).await.as_deref(),
            Some("legs heavy"),
            "bug: the athlete's own screen is no AI read"
        );
        assert_eq!(
            rows(withheld, true).await.as_deref(),
            Some("legs heavy"),
            "bug: the coach's own screen is no AI read"
        );
    }
}
