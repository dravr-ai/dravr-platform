// ABOUTME: Handler for predict_performance tool using VDOT and Riegel formulas
// ABOUTME: Predicts race times for 5K, 10K, half marathon, and marathon distances
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{
    NoRacePrediction, PredictionVdotSource, RacePrediction, RacePredictionDetail,
    RacePredictionResult,
};
use crate::implementations::stored_copy::{activities_or_stored, with_caveat, Reader, Served};
use crate::implementations::stored_physiology::stored_profile;
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use dravr_cageux::algorithms::PREDICTABLE_VDOT;
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::models::sport_alias::{resolve_sport_type, sport_matches_family};
use dravr_cageux::performance_prediction::{PerformancePredictor, VdotReading, VdotSource};
use dravr_cageux::physiological_constants::api_limits::DEFAULT_ACTIVITY_LIMIT;
use dravr_cageux::training_load::TrainingLoadCalculator;
use dravr_tronc::dates::{Timestamp, Zone};
use pierre_core::models::{Activity, SportType, UserPhysiologicalProfile};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_fitness_compute::AthleteInputs;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_services::athlete_clock::athlete_zone;
use pierre_services::athlete_vdot::{resolve_athlete_vdot, vdot_beyond_tables_warning};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tracing::warn;

/// Race distances `predictions_for_vdot` names, in metres.
fn race_distance_meters(name: &str) -> Option<f64> {
    match name {
        "5K" => Some(5_000.0),
        "10K" => Some(10_000.0),
        "15K" => Some(15_000.0),
        "Half Marathon" => Some(21_097.5),
        "Marathon" => Some(42_195.0),
        _ => None,
    }
}

/// The output's account of where `reading` came from, and the day it stands
/// for (none for a VDOT read back from the threshold pace).
fn describe_source(
    reading: &VdotReading,
    profile: Option<&UserPhysiologicalProfile>,
) -> (PredictionVdotSource, Option<Timestamp>) {
    match reading.source {
        VdotSource::Stated { as_of } => (
            PredictionVdotSource::Stated {
                as_of: as_of.utc_date().to_string(),
                method: profile
                    .and_then(|p| p.vdot.as_ref())
                    .and_then(|v| v.method.clone()),
            },
            Some(as_of),
        ),
        VdotSource::RecentRun {
            distance_meters,
            time_seconds,
            date,
        } => (
            PredictionVdotSource::RecentRun {
                distance_meters,
                time_seconds,
                pace_min_km: PerformancePredictor::format_pace_per_km(
                    distance_meters / time_seconds,
                ),
                date: date.to_rfc3339(),
            },
            Some(date),
        ),
        VdotSource::ThresholdPace {
            threshold_pace_sec_per_km,
        } => (
            PredictionVdotSource::ThresholdPace {
                threshold_pace_sec_per_km,
            },
            None,
        ),
    }
}

/// Predict race performance from the athlete's one VDOT
/// ([`resolve_athlete_vdot`]): a VDOT they stated under 12 weeks old, else
/// their best run of the last 6 weeks, else their saved threshold pace.
fn predict_race_performance(
    activities: &[Activity],
    target_sport: &str,
    athlete: &AthleteInputs,
    profile: Option<&UserPhysiologicalProfile>,
    algorithm_config: &AlgorithmConfig,
    zone: &Zone,
) -> RacePredictionResult {
    let now = Timestamp::now();
    let running_activities: Vec<&Activity> = activities
        .iter()
        .filter(|a| format!("{:?}", a.sport_type()).contains("Run"))
        .collect();

    let Some(reading) = resolve_athlete_vdot(profile, activities, now, algorithm_config) else {
        return RacePredictionResult::Unavailable(NoRacePrediction {
            target_sport: target_sport.to_owned(),
            message: "No VDOT to predict from: no race or VDOT stated in the last 12 weeks, no \
                      run of 3 km to 2 h in the last 6 weeks, and no saved threshold pace"
                .to_owned(),
            error: None,
            predictions: vec![],
        });
    };

    let mut predictions: Vec<RacePrediction> =
        PerformancePredictor::predictions_for_vdot(reading.vdot, algorithm_config)
            .into_iter()
            .filter_map(|(name, time_seconds)| {
                let distance_meters = race_distance_meters(&name)?;
                Some(RacePrediction {
                    predicted_pace_min_km: PerformancePredictor::format_pace_per_km(
                        distance_meters / time_seconds,
                    ),
                    predicted_time_formatted: PerformancePredictor::format_time(time_seconds),
                    predicted_time_seconds: time_seconds.round(),
                    distance: name,
                    distance_meters,
                })
            })
            .collect();
    if predictions.is_empty() {
        return RacePredictionResult::Unavailable(NoRacePrediction {
            target_sport: target_sport.to_owned(),
            message: "Unable to calculate race predictions for this VDOT".to_owned(),
            error: Some(format!(
                "VDOT {:.1} is outside the {:.0}-{:.0} range race times are predicted for",
                reading.vdot,
                PREDICTABLE_VDOT.start(),
                PREDICTABLE_VDOT.end()
            )),
            predictions: vec![],
        });
    }
    predictions.sort_by(|a, b| a.distance_meters.total_cmp(&b.distance_meters));

    let (vdot_source, basis_date) = describe_source(&reading, profile);
    let estimated = !matches!(reading.source, VdotSource::Stated { .. });
    let confidence = calculate_prediction_confidence(
        &running_activities,
        basis_date.as_ref(),
        athlete,
        algorithm_config,
        zone,
    );
    let mut notes = vec![
        "Predictions assume proper race preparation and taper".to_owned(),
        "Based on VDOT methodology by Jack Daniels".to_owned(),
        "Actual performance may vary with conditions and training".to_owned(),
    ];
    match reading.source {
        VdotSource::Stated { .. } => {}
        VdotSource::RecentRun { .. } => notes.push(
            "Estimate: the VDOT is read from the best run of the last 6 weeks, and a training \
             run reads low"
                .to_owned(),
        ),
        VdotSource::ThresholdPace { .. } => {
            notes.push("Estimate: the VDOT is read back from the saved threshold pace".to_owned());
        }
    }
    notes.extend(vdot_beyond_tables_warning(reading.vdot));

    RacePredictionResult::Predicted(Box::new(RacePredictionDetail {
        target_sport: target_sport.to_owned(),
        vdot: reading.vdot.round(),
        vdot_source,
        estimated,
        predictions,
        confidence,
        activities_analyzed: running_activities.len(),
        notes,
    }))
}

/// Calculate prediction confidence based on recency, training volume, and data quality
///
/// Confidence factors per B6 roadmap:
/// - Recency of the performance the VDOT stands for (< 30 days = high
///   confidence); a VDOT read back from the threshold pace has no date and
///   scores none
/// - Training volume (high CTL = more confidence), each run's stress scored
///   against `athlete`, the thresholds saved with `set_physiology`, the load
///   read as of the athlete's today in `zone`
/// - Number of recent races and consistency
#[allow(clippy::cast_precision_loss, clippy::bool_to_int_with_if)] // Multi-level threshold scoring, not simple boolean conversion
fn calculate_prediction_confidence(
    activities: &[&Activity],
    basis_date: Option<&Timestamp>,
    athlete: &AthleteInputs,
    algorithm_config: &AlgorithmConfig,
    zone: &Zone,
) -> String {
    use TrainingLoadCalculator;

    let now = Timestamp::now();
    // Factor 1: Recency (< 30 days = high confidence)
    let days_since_best = basis_date.map(|date| now.signed_duration_since(*date).num_days());
    let recency_score = match days_since_best {
        Some(days) if days < 30 => 2, // Recent performance
        Some(days) if days < 90 => 1, // Moderately recent
        _ => 0,                       // Old performance, or none dated
    };

    // Factor 2: Training volume (CTL)
    let mut owned_activities: Vec<_> = activities.iter().copied().cloned().collect();
    // Sort oldest-first — EMA calculation requires chronological order
    owned_activities.sort_by_key(Activity::start_date);
    let calculator =
        TrainingLoadCalculator::as_of_today(algorithm_config.clone(), zone.clone(), now);
    let ctl_score = if let Ok(training_load) = athlete.training_load(calculator, &owned_activities)
    {
        if training_load.ctl > 80.0 {
            2 // High training load
        } else if training_load.ctl > 40.0 {
            1 // Moderate training load
        } else {
            0 // Low training load
        }
    } else {
        0
    };

    // Factor 3: Number of activities
    let volume_score = if activities.len() >= 20 {
        2
    } else if activities.len() >= 10 {
        1
    } else {
        0
    };

    // Combine factors (max score = 6)
    let total_score = recency_score + ctl_score + volume_score;

    if total_score >= 5 {
        "high".to_owned()
    } else if total_score >= 3 {
        "medium".to_owned()
    } else {
        "low".to_owned()
    }
}

/// Handle `predict_performance` tool - predict future performance
pub(crate) fn handle_predict_performance(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;
        use DEFAULT_ACTIVITY_LIMIT;

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let target_sport = request
            .parameters
            .get("target_sport")
            .and_then(|v| v.as_str())
            .unwrap_or("Run");
        // VDOT models running and the prediction reads only runs, so another
        // sport is refused before any provider is read rather than answered
        // with running times labelled as that sport.
        if !resolve_sport_type(target_sport)
            .is_some_and(|sport| sport_matches_family(&sport, &SportType::Run))
        {
            return Err(ProtocolError::InvalidParameters(format!(
                "target_sport '{target_sport}' cannot be predicted: race-time prediction \
                 (VDOT) models running only; pass 'Run' or leave it out"
            )));
        }
        let provider_name = match resolve_provider_for_request(
            &request.parameters,
            executor,
            user_uuid,
            request.tenant_id.as_deref(),
        )
        .await
        {
            Ok(p) => p,
            Err(response) => return Ok(*response),
        };

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // The profile set_physiology saved: the thresholds every training-load
        // surface scores sessions against, and the stated VDOT and threshold
        // pace the VDOT is resolved from.
        let profile = stored_profile(&executor.resources, request.tenant_id.as_deref(), user_uuid)
            .await
            .map_err(|e| {
                ProtocolError::InternalError(format!(
                    "predict_performance: reading the saved thresholds failed: {e}"
                ))
            })?;
        let athlete = AthleteInputs::from_profile(profile.as_ref());

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                match activities_or_stored(
                    Reader::new(&executor.resources, user_uuid, request.tenant_id.as_deref()),
                    provider.as_ref(),
                    DEFAULT_ACTIVITY_LIMIT,
                )
                .await
                {
                    Ok(Served {
                        value: raw_activities,
                        caveat,
                    }) => {
                        // VDOT race-time prediction relies on best-effort
                        // samples; collapsing fragments to one row per session
                        // keeps the predictor from picking a doubled-recording
                        // PR as the canonical performance sample.
                        let (activities, _fragment_report) =
                            merge_duplicates(raw_activities, &DedupConfig::default());

                        let prediction = predict_race_performance(
                            &activities,
                            target_sport,
                            &athlete,
                            profile.as_ref(),
                            &executor.cageux_config().algorithms,
                            &athlete_zone(executor.resources.repos(), user_uuid).await,
                        );

                        with_caveat(
                            apply_format_typed(
                                UniversalResponse {
                                    success: true,
                                    result: None,
                                    error: None,
                                    metadata: Some({
                                        let mut map = HashMap::new();
                                        map.insert(
                                            "user_id".to_owned(),
                                            serde_json::Value::String(user_uuid.to_string()),
                                        );
                                        map
                                    }),
                                },
                                prediction,
                                output_format,
                            ),
                            caveat,
                        )
                    }
                    Err(e) => {
                        warn!(error = %e, "activity read for predictions failed");
                        Ok(UniversalResponse {
                            success: false,
                            result: None,
                            error: Some(format!(
                                "Failed to fetch activities: {}",
                                e.tool_message()
                            )),
                            metadata: None,
                        })
                    }
                }
            }
            Err(response) => Ok(*response),
        }
    })
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Elapsed;
    use pierre_core::models::{ActivityBuilder, ProfileVdot};
    use uuid::Uuid;

    use super::*;

    fn predict(
        profile: Option<&UserPhysiologicalProfile>,
        activities: &[Activity],
    ) -> RacePredictionResult {
        predict_race_performance(
            activities,
            "Run",
            &AthleteInputs::default(),
            profile,
            &AlgorithmConfig::default(),
            &Zone::utc(),
        )
    }

    fn stated_50() -> UserPhysiologicalProfile {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.vo2_max = Some(60.0);
        profile.vdot = Some(ProfileVdot {
            value: 50.0,
            as_of: (Timestamp::now() - Elapsed::days(10)).utc_date(),
            method: Some("race: 10 km in 41:21".to_owned()),
        });
        profile
    }

    #[test]
    fn a_stated_vdot_predicts_and_is_named_not_estimated() {
        let RacePredictionResult::Predicted(detail) = predict(Some(&stated_50()), &[]) else {
            panic!("a fresh stated VDOT predicts without any run");
        };
        assert!(
            (detail.vdot - 50.0).abs() < f64::EPSILON,
            "never the VO2max 60"
        );
        assert!(!detail.estimated, "a stated VDOT is not an estimate");
        assert!(
            matches!(
                &detail.vdot_source,
                PredictionVdotSource::Stated { method: Some(m), .. } if m == "race: 10 km in 41:21"
            ),
            "the source names the stated VDOT and its method"
        );
        let names: Vec<&str> = detail
            .predictions
            .iter()
            .map(|p| p.distance.as_str())
            .collect();
        assert_eq!(
            names,
            ["5K", "10K", "15K", "Half Marathon", "Marathon"],
            "every distance, shortest first"
        );
        assert!(
            detail
                .predictions
                .iter()
                .all(|p| p.predicted_pace_min_km != "N/A"),
            "every distance carries its pace"
        );
    }

    #[test]
    fn a_recent_run_predicts_as_a_labelled_estimate() {
        // 10 km in 45:00 last week reads VDOT 45.3.
        let run = ActivityBuilder::new(
            "run".to_owned(),
            "10 km".to_owned(),
            SportType::Run,
            Timestamp::now() - Elapsed::days(7),
            2700,
            "test".to_owned(),
        )
        .distance_meters(10_000.0)
        .build();
        let RacePredictionResult::Predicted(detail) = predict(None, &[run]) else {
            panic!("a recent run predicts");
        };
        assert!(
            (detail.vdot - 45.0).abs() < f64::EPSILON,
            "bug: 10 km in 45:00 must predict from VDOT 45 (rounded), got {}",
            detail.vdot
        );
        assert!(detail.estimated, "a run read as a race is an estimate");
        assert!(
            matches!(detail.vdot_source, PredictionVdotSource::RecentRun { .. }),
            "bug: a VDOT read from the recent run must name the run as its source"
        );
        assert!(
            detail.notes.iter().any(|n| n.starts_with("Estimate")),
            "the notes label the estimate: {:?}",
            detail.notes
        );
    }

    #[test]
    fn a_vdot_below_daniels_tables_predicts_with_a_warning() {
        // 5 km in 35:00 is VDOT 25.6: race times are predicted from 20 to 90,
        // and below Daniels' 30-85 the notes say they are less certain.
        let mut profile = stated_50();
        if let Some(vdot) = profile.vdot.as_mut() {
            vdot.value = 25.6;
        }
        let RacePredictionResult::Predicted(detail) = predict(Some(&profile), &[]) else {
            panic!("bug: VDOT 25.6 must predict race times");
        };
        assert_eq!(
            detail.predictions.len(),
            5,
            "bug: every distance is predicted at VDOT 25.6"
        );
        assert!(
            detail.notes.iter().any(|n| n.contains("less certain")),
            "bug: the notes must say a VDOT below the tables is less certain: {:?}",
            detail.notes
        );
    }

    #[test]
    fn no_performance_predicts_nothing() {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.vo2_max = Some(60.0);
        assert!(
            matches!(
                predict(Some(&profile), &[]),
                RacePredictionResult::Unavailable(NoRacePrediction { error: None, .. })
            ),
            "a VO2max alone predicts no race"
        );
    }
}
