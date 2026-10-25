// ABOUTME: Handler for calculate_metrics tool computing pace, speed, distance per climb and the HR reading
// ABOUTME: Supports both inline activity parameters and provider-fetched activity_id paths
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::implementations::analytics::output::{ActivityMetricsResult, MetricsInputSummary};
use crate::implementations::configuration::configured_max_hr_algorithm;
use crate::implementations::stored_physiology::stored_profile;
use crate::protocol::format::{apply_format_typed, extract_output_format};
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use dravr_cageux::metrics::distance_per_climb_score;
use dravr_cageux::models::maxhr::MaxHrAlgorithm;
use dravr_cageux::physiological_constants::unit_conversions::MS_TO_KMH_FACTOR;
use dravr_cageux::zones::{HeartRateReading, HrZones};
use pierre_config::constants::limits::{self, METERS_PER_KILOMETER};
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::UserPhysiologicalProfile;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_formatters::OutputFormat;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Activity parameters extracted from request
struct ActivityParameters {
    distance: f64,
    duration: u64,
    elevation_gain: f64,
    heart_rate: Option<u32>,
    max_hr_provided: Option<f64>,
    user_age: Option<u32>,
}

/// Calculated fitness metrics
struct CalculatedMetrics {
    pace: f64,
    speed: f64,
    /// Metres covered per metre climbed (cageux), or why there is none.
    distance_per_climb: Result<f64, String>,
}

/// The maximum heart rate the zones are cut to, and where it came from.
struct MaxHeartRate {
    bpm: f64,
    source: String,
}

/// Parse activity parameters from request
///
/// Extracts activity metrics (distance, duration, elevation, heart rate) and
/// user profile data (max HR, age) from the MCP request parameters.
///
/// # Arguments
/// * `request` - The incoming MCP request with parameters
///
/// # Returns
/// Parsed activity parameters or error if required fields are missing
///
/// # Errors
/// Returns `ProtocolError::InvalidRequest` if activity parameter is missing
fn parse_activity_parameters(
    request: &UniversalRequest,
) -> Result<ActivityParameters, ProtocolError> {
    let activity = request.parameters.get("activity").ok_or_else(|| {
        ProtocolError::InvalidRequest("activity parameter is required".to_owned())
    })?;

    let distance = activity
        .get("distance")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);

    let duration = activity
        .get("duration")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let elevation_gain = activity
        .get("elevation_gain")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);

    let heart_rate = activity
        .get("average_heart_rate")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());

    let max_hr_provided = request
        .parameters
        .get("max_hr")
        .and_then(serde_json::Value::as_f64);

    let user_age = request
        .parameters
        .get("age")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());

    Ok(ActivityParameters {
        distance,
        duration,
        elevation_gain,
        heart_rate,
        max_hr_provided,
        user_age,
    })
}

/// Determine maximum heart rate
///
/// This call's `max_hr` first, then the saved maximum, else cageux's estimate
/// from the age (this call's, else the saved one) by `algorithm`, the
/// deployment's configured max-HR estimator; else none, and the heart-rate
/// reading is absent rather than read against a stranger's maximum.
fn determine_max_heart_rate(
    max_hr_provided: Option<f64>,
    user_age: Option<u32>,
    profile: Option<&UserPhysiologicalProfile>,
    algorithm: MaxHrAlgorithm,
) -> Option<MaxHeartRate> {
    if let Some(bpm) = max_hr_provided {
        return Some(MaxHeartRate {
            bpm,
            source: "provided".to_owned(),
        });
    }
    if let Some(bpm) = profile.and_then(|p| p.max_hr) {
        return Some(MaxHeartRate {
            bpm: f64::from(bpm),
            source: "profile".to_owned(),
        });
    }
    let age = user_age.or_else(|| profile.and_then(|p| p.age).map(u32::from))?;
    let bpm = algorithm.estimate(age, None).ok()?;
    Some(MaxHeartRate {
        bpm,
        source: format!("estimated_from_age_{age}_{}", algorithm.name()),
    })
}

/// Calculate activity metrics from parameters
///
/// Computes pace (min/km), speed (km/h) and cageux's distance-per-climb
/// score, which is absent with its reason when the distance or the climb is
/// unrecorded. Uses defensive checks to avoid division by zero.
///
/// # Arguments
/// * `params` - Activity parameters (distance, duration, elevation, HR)
///
/// # Returns
/// Calculated metrics structure
fn calculate_activity_metrics(params: &ActivityParameters) -> CalculatedMetrics {
    let duration_f64 =
        f64::from(u32::try_from(params.duration.min(u64::from(u32::MAX))).unwrap_or(u32::MAX));

    let pace = if params.distance > 0.0 && params.duration > 0 {
        duration_f64 / (params.distance / METERS_PER_KILOMETER)
    } else {
        0.0
    };

    let speed = if params.duration > 0 {
        (params.distance / duration_f64) * MS_TO_KMH_FACTOR
    } else {
        0.0
    };

    let distance_per_climb =
        distance_per_climb_score(Some(params.distance), Some(params.elevation_gain))
            .map_err(|reason| reason.to_string());

    CalculatedMetrics {
        pace,
        speed,
        distance_per_climb,
    }
}

/// The activity's average heart rate read on the athlete's zones, or why it
/// cannot be.
///
/// The zones are cageux's [`HrZones::for_athlete`] from the saved resting
/// heart rate and `VO2max` and the maximum [`determine_max_heart_rate`]
/// resolved.
fn heart_rate_reading(
    heart_rate: Option<u32>,
    profile: Option<&UserPhysiologicalProfile>,
    max_hr: Option<&MaxHeartRate>,
) -> Result<HeartRateReading, String> {
    let heart_rate = heart_rate.ok_or_else(|| "the activity carried no heart rate".to_owned())?;
    let resting_hr = profile.and_then(|p| p.resting_hr);
    let max_bpm = max_hr.and_then(|max| {
        // Safe: a maximum heart rate is a few hundred bpm at most.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bpm = max.bpm.round() as u16;
        (max.bpm.is_finite() && max.bpm > 0.0).then_some(bpm)
    });
    let (Some(resting_hr), Some(max_hr)) = (resting_hr, max_bpm) else {
        return Err(HrZonesUnavailable::MissingHeartRates {
            resting_hr: resting_hr.is_none(),
            max_hr: max_bpm.is_none(),
        }
        .reason());
    };
    let zones = HrZones::for_athlete(profile.and_then(|p| p.vo2_max), resting_hr, max_hr)
        .map_err(|e| HrZonesUnavailable::Rejected(e.to_string()).reason())?;
    Ok(zones.reading(u16::try_from(heart_rate).unwrap_or(u16::MAX)))
}

/// Build metrics calculation response
///
/// Constructs the MCP response with calculated metrics, summary data, and metadata.
/// Includes timestamps and personalization flags for transparency.
///
/// # Arguments
/// * `params` - Original activity parameters
/// * `metrics` - Calculated metrics (pace, speed, distance per climb)
/// * `max_hr` - Determined maximum heart rate and its source, when known
///
/// # Returns
/// Complete `UniversalResponse` with results and metadata
fn build_metrics_response(
    params: &ActivityParameters,
    metrics: &CalculatedMetrics,
    max_hr: Option<&MaxHeartRate>,
    reading: Result<HeartRateReading, String>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    use limits;

    let payload = ActivityMetricsResult {
        pace: metrics.pace,
        speed: metrics.speed,
        distance_per_climb_score: metrics.distance_per_climb.as_ref().ok().copied(),
        distance_per_climb_unavailable: metrics.distance_per_climb.as_ref().err().cloned(),
        max_hr_used: max_hr.map(|max| max.bpm),
        max_hr_source: max_hr.map_or_else(|| "unknown".to_owned(), |max| max.source.clone()),
        heart_rate_unavailable: reading.as_ref().err().cloned(),
        heart_rate_reading: reading.ok(),
        metrics_summary: MetricsInputSummary {
            distance_km: params.distance / METERS_PER_KILOMETER,
            duration_minutes: params.duration / limits::SECONDS_PER_MINUTE,
            elevation_meters: params.elevation_gain,
            average_heart_rate: params.heart_rate,
        },
    };

    let response = UniversalResponse {
        success: true,
        result: None,
        error: None,
        metadata: Some({
            let mut map = HashMap::new();
            map.insert(
                "calculation_timestamp".into(),
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
            );
            map.insert(
                "metric_version".into(),
                serde_json::Value::String("2.0".into()),
            );
            map.insert(
                "personalized".into(),
                serde_json::Value::Bool(
                    params.max_hr_provided.is_some() || params.user_age.is_some(),
                ),
            );
            map
        }),
    };

    apply_format_typed(response, payload, output_format)
}

/// Fetch activity from provider and calculate metrics (helper for `activity_id` path)
async fn fetch_and_calculate_metrics(
    executor: &UniversalToolExecutor,
    request: &UniversalRequest,
    activity_id: &str,
    provider_name: &str,
    user_uuid: uuid::Uuid,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    // Authenticate through the chokepoint every provider read goes through;
    // its refusal is the answer, reconnect signal included.
    let provider = match executor
        .auth_service
        .create_authenticated_provider(provider_name, user_uuid, request.tenant_id.as_deref())
        .await
    {
        Ok(provider) => provider,
        Err(response) => return Ok(*response),
    };

    // Fetch activity from provider
    let activity = provider.get_activity(activity_id).await.map_err(|e| {
        ProtocolError::ExecutionFailed(format!("Failed to fetch activity {activity_id}: {e}"))
    })?;

    // Convert Activity model to parameters format
    let mut request_with_activity = request.clone();
    if let Some(params_obj) = request_with_activity.parameters.as_object_mut() {
        params_obj.insert(
            "activity".to_owned(),
            serde_json::json!({
                "distance": activity.distance_meters(),
                "duration": activity.duration_seconds(),
                "elevation_gain": activity.elevation_gain(),
                "average_heart_rate": activity.average_heart_rate(),
            }),
        );
    } else {
        return Err(ProtocolError::InvalidParameters(
            "parameters must be a JSON object".to_owned(),
        ));
    }

    // Parse parameters from converted activity
    let params = parse_activity_parameters(&request_with_activity)?;
    let algorithm =
        configured_max_hr_algorithm(&executor.resources, user_uuid, request.tenant_id.as_deref())
            .await
            .map_err(|e| {
                ProtocolError::InternalError(format!(
                    "calculate_metrics: reading the configured max-HR algorithm failed: {e}"
                ))
            })?;
    let profile = stored_profile(&executor.resources, request.tenant_id.as_deref(), user_uuid)
        .await
        .map_err(|e| {
            ProtocolError::InternalError(format!(
                "calculate_metrics: reading the athlete's physiology failed: {e}"
            ))
        })?;
    let max_hr = determine_max_heart_rate(
        params.max_hr_provided,
        params.user_age,
        profile.as_ref(),
        algorithm,
    );
    let metrics = calculate_activity_metrics(&params);
    let reading = heart_rate_reading(params.heart_rate, profile.as_ref(), max_hr.as_ref());

    build_metrics_response(&params, &metrics, max_hr.as_ref(), reading, output_format)
}

/// Handle `calculate_metrics` tool - calculate custom fitness metrics (async)
///
/// # Errors
/// Returns `ProtocolError` if activity parameter is missing or calculation fails
pub(crate) fn handle_calculate_metrics(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // `activity_id` and `provider` are both declared required by this tool's
        // schema, so this is the only way in. A second branch used to accept an
        // `activity` object inline and skip the fetch, but that shape appears
        // nowhere in the schema — no caller following the tool definition could
        // know to send it, and the only thing that ever did was a test. The
        // shared helpers below it stay: `fetch_and_calculate_metrics` builds the
        // same `activity` object from the fetched activity and runs them.
        let activity_id = request
            .parameters
            .get("activity_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProtocolError::InvalidParameters("activity_id parameter is required".to_owned())
            })?;

        let provider_name = request
            .parameters
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProtocolError::InvalidParameters(
                    "provider parameter required when using activity_id".to_owned(),
                )
            })?;

        fetch_and_calculate_metrics(
            executor,
            &request,
            activity_id,
            provider_name,
            user_uuid,
            output_format,
        )
        .await
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use dravr_cageux::zones::HrIntensity;
    use pierre_core::models::SportType;
    use uuid::Uuid;

    use super::*;

    fn profile(resting_hr: Option<u16>, max_hr: Option<u16>) -> UserPhysiologicalProfile {
        let mut profile = UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Run);
        profile.resting_hr = resting_hr;
        profile.max_hr = max_hr;
        profile
    }

    #[test]
    fn the_heart_rate_is_read_on_the_athletes_zones() -> Result<(), Box<dyn Error>> {
        let athlete = profile(Some(50), Some(190));
        let max = determine_max_heart_rate(None, None, Some(&athlete), MaxHrAlgorithm::Tanaka)
            .ok_or("the saved maximum")?;
        assert_eq!(max.source, "profile");
        let reading = heart_rate_reading(Some(165), Some(&athlete), Some(&max))?;
        // cageux's zones for resting 50 / max 190 (no VO2max: Uth's ratio).
        let expected = HrZones::for_athlete(None, 50, 190)?.reading(165);
        assert_eq!(reading, expected);
        assert_eq!(reading.zone, 4);
        assert_eq!(reading.intensity, HrIntensity::Hard);
        assert!((reading.reserve_fraction - 115.0 / 140.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn a_maximum_given_or_estimated_from_age_cuts_the_zones() -> Result<(), Box<dyn Error>> {
        let athlete = profile(Some(50), None);
        let given =
            determine_max_heart_rate(Some(200.0), None, Some(&athlete), MaxHrAlgorithm::Tanaka)
                .ok_or("given")?;
        assert_eq!(given.source, "provided");
        let aged = determine_max_heart_rate(None, Some(40), Some(&athlete), MaxHrAlgorithm::Tanaka)
            .ok_or("estimated")?;
        // Tanaka: 208 - 0.7 x 40 = 180.
        assert!((aged.bpm - 180.0).abs() < 1e-9);
        assert_eq!(aged.source, "estimated_from_age_40_tanaka");
        let reading = heart_rate_reading(Some(150), Some(&athlete), Some(&aged))?;
        assert_eq!(reading, HrZones::for_athlete(None, 50, 180)?.reading(150));
        Ok(())
    }

    fn parameters(distance: f64, elevation_gain: f64) -> ActivityParameters {
        ActivityParameters {
            distance,
            duration: 3600,
            elevation_gain,
            heart_rate: None,
            max_hr_provided: None,
            user_age: None,
        }
    }

    #[test]
    fn the_climb_score_is_cageuxs_or_absent_with_its_reason() -> Result<(), Box<dyn Error>> {
        let hilly = calculate_activity_metrics(&parameters(10_000.0, 250.0));
        assert_eq!(
            hilly.distance_per_climb,
            Ok(distance_per_climb_score(Some(10_000.0), Some(250.0))?)
        );
        // A flat activity records no climb: no score, never a fixed value.
        let flat = calculate_activity_metrics(&parameters(10_000.0, 0.0));
        assert_eq!(
            flat.distance_per_climb,
            Err("the activity records no elevation gain".to_owned())
        );
        let stationary = calculate_activity_metrics(&parameters(0.0, 100.0));
        assert_eq!(
            stationary.distance_per_climb,
            Err("the activity records no distance".to_owned())
        );
        Ok(())
    }

    #[test]
    fn without_zones_there_is_no_reading_and_the_reason_says_why() {
        let no_resting = profile(None, Some(190));
        let max = determine_max_heart_rate(None, None, Some(&no_resting), MaxHrAlgorithm::Tanaka);
        let reason = heart_rate_reading(Some(150), Some(&no_resting), max.as_ref()).unwrap_err();
        assert!(reason.contains("resting heart rate"), "{reason}");

        let nobody = determine_max_heart_rate(None, None, None, MaxHrAlgorithm::Tanaka);
        assert!(nobody.is_none(), "no maximum is invented");
        let reason = heart_rate_reading(Some(150), None, None).unwrap_err();
        assert!(reason.contains("resting and maximum"), "{reason}");

        let reason =
            heart_rate_reading(None, Some(&profile(Some(50), Some(190))), None).unwrap_err();
        assert_eq!(reason, "the activity carried no heart rate");
    }
}
