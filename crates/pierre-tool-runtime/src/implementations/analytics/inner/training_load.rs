// ABOUTME: Handler for analyze_training_load tool with CTL/ATL/TSB metrics
// ABOUTME: Calculates chronic/acute training load and stress balance with optional sleep recovery context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::format::{apply_format_to_response, extract_output_format};
use crate::protocol::provider_helpers::resolve_provider_for_request;
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
#[cfg(feature = "client-notifications")]
use crate::runtime::ToolRuntime;
use pierre_core::models::Activity;
use pierre_core::models::FormBand;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_intelligence::{AlgorithmConfig, SleepAnalyzer, TrainingLoadCalculator, TssDataPoint};
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;
#[cfg(feature = "client-notifications")]
use pierre_notifications::TenantId;
use pierre_providers::deduplication::{dedupe_and_report, DedupConfig};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
#[cfg(feature = "client-notifications")]
use std::sync::Arc;
use tracing::{debug, warn};
#[cfg(feature = "client-notifications")]
use uuid::Uuid;

/// User physiological parameters for personalized TSS calculation
pub struct UserPhysiologicalParams {
    /// Functional Threshold Power, watts
    pub ftp: Option<f64>,
    /// Lactate Threshold Heart Rate, bpm
    pub lthr: Option<f64>,
    /// Maximum heart rate, bpm
    pub max_hr: Option<f64>,
    /// Resting heart rate, bpm
    pub resting_hr: Option<f64>,
    /// Body mass, kilograms
    pub weight_kg: Option<f64>,
}

/// Fetch user physiological parameters from stored configuration.
///
/// Returns params with whatever the user has configured; missing values stay None
/// and the TSS calculator falls back to pace-based estimation.
async fn fetch_user_physiological_params(
    executor: &UniversalToolExecutor,
    user_uuid: uuid::Uuid,
) -> UserPhysiologicalParams {
    let config_json = executor
        .resources
        .repos()
        .profiles
        .get_configuration(&user_uuid.to_string())
        .await
        .ok()
        .flatten();

    let Some(config_str) = config_json else {
        return UserPhysiologicalParams {
            ftp: None,
            lthr: None,
            max_hr: None,
            resting_hr: None,
            weight_kg: None,
        };
    };

    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    // Configuration is stored as: { "profile": { ... }, "session_overrides": { ... } }
    // Physiological data lives in session_overrides or at the top level.
    let overrides = config.get("session_overrides").unwrap_or(&config);

    UserPhysiologicalParams {
        ftp: overrides.get("ftp").and_then(serde_json::Value::as_f64),
        lthr: overrides
            .get("lactate_threshold_hr")
            .or_else(|| overrides.get("threshold_hr"))
            .and_then(serde_json::Value::as_f64),
        max_hr: overrides.get("max_hr").and_then(serde_json::Value::as_f64),
        resting_hr: overrides
            .get("resting_hr")
            .and_then(serde_json::Value::as_f64),
        weight_kg: overrides
            .get("weight_kg")
            .or_else(|| overrides.get("weight"))
            .and_then(serde_json::Value::as_f64),
    }
}

/// Recovery context from sleep/HRV data for training load interpretation
struct RecoveryContextInfo {
    /// Sleep quality score (0-100)
    sleep_quality_score: f64,
    /// Recovery status interpretation
    recovery_status: String,
    /// HRV RMSSD if available
    hrv_rmssd: Option<f64>,
    /// Sleep duration in hours
    sleep_hours: f64,
}

/// Fetch recovery context for training load analysis
///
/// Fetches recent sleep data and provides recovery context to interpret
/// training load data (CTL/ATL/TSB) more accurately.
async fn fetch_recovery_context_for_training_load(
    executor: &UniversalToolExecutor,
    user_uuid: uuid::Uuid,
    tenant_id: Option<&str>,
    sleep_provider_name: &str,
) -> Result<RecoveryContextInfo, String> {
    use crate::protocol::sleep_helpers::fetch_provider_sleep_data;
    use SleepAnalyzer;

    // Fetch sleep data from provider
    let sleep_data =
        fetch_provider_sleep_data(executor, user_uuid, tenant_id, sleep_provider_name, 1)
            .await
            .map_err(|e| e.error.unwrap_or_else(|| "Unknown error".to_owned()))?;

    // Calculate sleep quality score
    let cageux_config = executor.cageux_config();
    let config = &cageux_config.sleep_recovery;
    let sleep_quality = SleepAnalyzer::calculate_sleep_quality(&sleep_data, config)
        .map_err(|e| format!("Sleep quality calculation failed: {e}"))?;

    // Determine recovery status based on sleep quality
    let recovery_status = if sleep_quality.overall_score >= 90.0 {
        "excellent".to_owned()
    } else if sleep_quality.overall_score >= 75.0 {
        "good".to_owned()
    } else if sleep_quality.overall_score >= 60.0 {
        "moderate".to_owned()
    } else if sleep_quality.overall_score >= 40.0 {
        "fair".to_owned()
    } else {
        "poor".to_owned()
    };

    Ok(RecoveryContextInfo {
        sleep_quality_score: sleep_quality.overall_score,
        recovery_status,
        hrv_rmssd: sleep_data.hrv_rmssd_ms,
        sleep_hours: sleep_data.duration_hours,
    })
}

/// Analyze detailed training load from activities using user-specific physiological data.
///
/// Public so the payload it builds — `form_band`, `form_assessment`,
/// `tsb_pct_of_ctl` and the banded `taper_status` — has content coverage. The
/// tool handler that wraps it sources activities from a connected provider, so a
/// protocol-level test on an unconnected fixture can only ever exercise the
/// error arm and asserts nothing about these fields.
pub fn analyze_detailed_training_load(
    activities: &[Activity],
    timeframe: &str,
    params: &UserPhysiologicalParams,
    algorithm_config: &AlgorithmConfig,
) -> serde_json::Value {
    use TrainingLoadCalculator;

    if activities.is_empty() {
        return serde_json::json!({
            "timeframe": timeframe,
            "message": "No activities found for training load analysis",
        });
    }

    // Sort activities oldest-first — the EMA calculation in TrainingLoadCalculator
    // requires chronological order (it computes days_span = last_date - first_date
    // and returns 0 if negative). Strava returns activities newest-first.
    let mut sorted_activities = activities.to_vec();
    sorted_activities.sort_by_key(Activity::start_date);

    let calculator = TrainingLoadCalculator::from_config(algorithm_config.clone());

    // Pass user physiological data for accurate TSS calculation.
    // When present, enables power-based (FTP) or HR-based (LTHR) TSS
    // instead of the less accurate pace-based fallback.
    let Ok(training_load) = calculator.calculate_training_load(
        &sorted_activities,
        params.ftp,
        params.lthr,
        params.max_hr,
        params.resting_hr,
        params.weight_kg,
    ) else {
        return serde_json::json!({
            "timeframe": timeframe,
            "message": "Unable to calculate training load - insufficient activity data",
        });
    };

    let ctl = training_load.ctl;
    let atl = training_load.atl;
    let tsb = training_load.tsb;
    // Form as % of CTL, and the single band derived from it. Every wording in
    // this payload comes off `band`, so the response cannot tell the athlete
    // that one number is both a normal training block and an elevated risk.
    let form_pct = FormBand::form_pct(tsb, ctl);
    let band = FormBand::from_form_pct(form_pct);

    // Calculate weekly TSS totals from TSS history
    let weekly_tss = calculate_weekly_tss_from_history(&training_load.tss_history);

    let taper_recommendation = taper_status(band);

    // Periodization suggestions
    let mut periodization_suggestions = Vec::new();
    if atl > ctl * 1.5 {
        periodization_suggestions
            .push("Recent spike in training - allow adaptation time".to_owned());
    } else if ctl > 30.0 && atl < ctl * 0.8 {
        periodization_suggestions
            .push("Well adapted to current load - room to build gradually".to_owned());
    }
    if ctl < 30.0 {
        periodization_suggestions
            .push("Building base - focus on consistency and volume".to_owned());
    } else if ctl > 80.0 {
        periodization_suggestions
            .push("High fitness level - maintain or add recovery weeks".to_owned());
    }

    serde_json::json!({
        "timeframe": timeframe,
        "load_metrics": {
            "ctl": ctl.round(),
            "atl": atl.round(),
            "tsb": tsb.round(),
            "tsb_pct_of_ctl": form_pct.map(f64::round),
            "weekly_tss": weekly_tss,
        },
        "form_band": band,
        "form_assessment": band.label(),
        "taper_status": taper_recommendation,
        "periodization_suggestions": periodization_suggestions,
        "training_zones": classify_training_load(ctl),
        "recommendations": generate_load_recommendations(ctl, band, form_pct),
        "activities_analyzed": training_load.tss_history.len(),
        "interpretation": {
            "ctl": "Chronic Training Load - fitness level (42-day average TSS)",
            "atl": "Acute Training Load - fatigue level (7-day average TSS)",
            "tsb": "Training Stress Balance - form (CTL - ATL); interpret via tsb_pct_of_ctl, not the raw number",
            "tsb_pct_of_ctl": "Form relative to this athlete's own fitness. null when there is no chronic base to normalize against, in which case form cannot be judged at all",
            "form_band": "The band tsb_pct_of_ctl falls in: insufficient_history when tsb_pct_of_ctl is null, deep_fatigue below -30%, heavy_block -30% to -20%, productive -20% to -10%, balanced -10% to +5%, fresh +5% to +20%, detraining above +20%. Describes fatigue relative to fitness; it is not an injury prediction",
        },
    })
}

/// Calculate weekly TSS totals from `TssDataPoint` history (Phase 1 format)
fn calculate_weekly_tss_from_history(tss_history: &[TssDataPoint]) -> Vec<serde_json::Value> {
    use HashMap;

    if tss_history.is_empty() {
        return Vec::new();
    }

    // Group by week
    let mut weekly_totals: HashMap<i32, f64> = HashMap::new();
    let first_date = tss_history[0].date;

    for point in tss_history {
        let days_diff = (point.date - first_date).num_days();
        #[allow(clippy::cast_possible_truncation)]
        let week_number_i32 = (days_diff / 7) as i32;
        *weekly_totals.entry(week_number_i32).or_insert(0.0) += point.tss;
    }

    // Convert to sorted vec
    let mut weeks: Vec<(i32, f64)> = weekly_totals.into_iter().collect();
    weeks.sort_by_key(|(week, _)| *week);

    weeks
        .iter()
        .map(|(week, tss)| {
            serde_json::json!({
                "week": week,
                "total_tss": tss.round(),
            })
        })
        .collect()
}

/// Taper reading for each form band.
///
/// Shares [`FormBand`]'s edges rather than re-deriving them, so the taper
/// advice cannot disagree with the band the same response reports.
const fn taper_status(band: FormBand) -> &'static str {
    match band {
        FormBand::InsufficientHistory => "Not enough chronic history to judge taper status",
        FormBand::DeepFatigue => {
            "Deep fatigue relative to fitness - reduce volume, keep short intensity"
        }
        FormBand::HeavyBlock | FormBand::Productive => {
            "Productive training zone - taper before racing, not before training"
        }
        FormBand::Balanced => "Close to fresh - a light taper reaches race form",
        FormBand::Fresh => "Race-ready form - well tapered",
        FormBand::Detraining => "Very fresh - possibly detrained, sharpen with intensity",
    }
}

/// Classify training load level
fn classify_training_load(ctl: f64) -> serde_json::Value {
    let level = if ctl < 25.0 {
        "Beginner"
    } else if ctl < 45.0 {
        "Intermediate"
    } else if ctl < 70.0 {
        "Advanced"
    } else if ctl < 100.0 {
        "Elite"
    } else {
        "Very High"
    };

    serde_json::json!({
        "level": level,
        "ctl_range": match level {
            "Beginner" => "< 25",
            "Intermediate" => "25-45",
            "Advanced" => "45-70",
            "Elite" => "70-100",
            _ => "> 100",
        },
    })
}

/// Generate load-specific recommendations from the athlete's form band.
///
/// Descriptive, banded on the athlete's own chronic load — never absolute
/// TSB thresholds, which misread elite athletes' normal training blocks
/// as emergencies. `form_pct` is carried only to quote the number back.
fn generate_load_recommendations(ctl: f64, band: FormBand, form_pct: Option<f64>) -> Vec<String> {
    let mut recommendations = Vec::new();

    // Deep fatigue is the only band that asks the athlete to back off, and it
    // splits by depth; every other band is normal training or freshness.
    match (band, form_pct) {
        (FormBand::DeepFatigue, Some(pct)) if pct < -40.0 => {
            recommendations.push(format!(
                "Form is {pct:.0}% of fitness - deepest fatigue band; reduce volume and reassess in 2-3 days"
            ));
        }
        (FormBand::DeepFatigue, Some(pct)) => {
            recommendations.push(format!(
                "Form is {pct:.0}% of fitness - past the productive zone; favor recovery over added volume"
            ));
        }
        (FormBand::HeavyBlock, Some(pct)) => {
            recommendations.push(format!(
                "Form is {pct:.0}% of fitness - the deep end of a productive block; hold the block, keep recovery honest"
            ));
        }
        (FormBand::Detraining, Some(pct)) => {
            recommendations.push(format!(
                "Form is +{pct:.0}% of fitness - very fresh; good window for a breakthrough workout or race"
            ));
        }
        (FormBand::InsufficientHistory, _) => {
            recommendations.push(
                "Not enough chronic training history to judge form - keep logging sessions before reading TSB".to_owned(),
            );
        }
        _ => {}
    }

    // Progressive load recommendations
    if ctl < 30.0 {
        recommendations
            .push("Building base - grow weekly TSS gradually (3-5 points/week)".to_owned());
    } else if ctl > 80.0 {
        recommendations.push(
            "High chronic load - schedule periodic recovery weeks (reduce 20-30%)".to_owned(),
        );
    }

    if recommendations.is_empty() {
        recommendations
            .push("Training load is well balanced - maintain current approach".to_owned());
    }

    recommendations
}

/// Handle `analyze_training_load` tool - analyze training load metrics (CTL/ATL/TSB)
#[must_use]
pub fn handle_analyze_training_load(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        use parse_user_id_for_protocol;

        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "analyze_training_load cancelled by user".to_owned(),
                ));
            }
        }

        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let provider_name = match resolve_provider_for_request(
            &request.parameters,
            executor,
            user_uuid,
            request.tenant_id.as_deref(),
        )
        .await
        {
            Ok(p) => p,
            Err(response) => return Ok(response),
        };
        let timeframe = request
            .parameters
            .get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("week");

        // Extract optional sleep_provider for cross-provider recovery analysis
        let sleep_provider = request
            .parameters
            .get("sleep_provider")
            .and_then(|v| v.as_str());

        // Extract output format parameter: "json" (default) or "toon"
        let output_format = extract_output_format(&request);

        // Report progress - starting authentication
        if let Some(reporter) = &request.progress_reporter {
            reporter.report(
                20.0,
                Some(100.0),
                Some("Checking authentication...".to_owned()),
            );
        }

        // Check cancellation before auth
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "analyze_training_load cancelled before authentication".to_owned(),
                ));
            }
        }

        match executor
            .auth_service
            .create_authenticated_provider(&provider_name, user_uuid, request.tenant_id.as_deref())
            .await
        {
            Ok(provider) => {
                // Report progress after auth
                if let Some(reporter) = &request.progress_reporter {
                    reporter.report(
                        40.0,
                        Some(100.0),
                        Some("Authenticated - fetching activities...".to_owned()),
                    );
                }

                // Check cancellation before provider creation
                if let Some(token) = &request.cancellation_token {
                    if token.is_cancelled().await {
                        return Err(ProtocolError::OperationCancelled(
                            "analyze_training_load cancelled before fetch".to_owned(),
                        ));
                    }
                }

                let activity_limit = executor.resources.config().activity_fetch_limit;
                match provider.get_activities(Some(activity_limit), None).await {
                    Ok(raw_activities) => {
                        // Collapse overlapping GPS recordings (Garmin auto-split,
                        // dual-device, Strava re-upload) into canonical sessions
                        // before TSS / CTL / ATL are computed; otherwise the
                        // chronic load curve double-counts every fragmented
                        // workout and trips overtraining notifications on
                        // synthetic volume.
                        let (activities, fragment_report) =
                            dedupe_and_report(&raw_activities, &DedupConfig::default());
                        if fragment_report.has_fragments() {
                            debug!(
                                raw = fragment_report.raw_count,
                                sessions = fragment_report.session_count,
                                groups = fragment_report.groups.len(),
                                "training_load: applied fragment dedup",
                            );
                        }
                        // Report progress before analysis
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                70.0,
                                Some(100.0),
                                Some(format!(
                                    "Analyzing training load for {} activities...",
                                    activities.len()
                                )),
                            );
                        }

                        // Fetch user physiological data for accurate TSS
                        let physio_params =
                            fetch_user_physiological_params(executor, user_uuid).await;

                        let mut analysis = analyze_detailed_training_load(
                            &activities,
                            timeframe,
                            &physio_params,
                            &executor.cageux_config().algorithms,
                        );

                        // Fire intelligence-driven notification triggers based on computed metrics
                        #[cfg(feature = "client-notifications")]
                        fire_training_load_notifications(
                            &executor.resources,
                            user_uuid,
                            request.tenant_id.as_deref(),
                            &analysis,
                        );

                        // If sleep_provider is specified, fetch recovery context
                        let recovery_context = if let Some(sleep_provider_name) = sleep_provider {
                            match fetch_recovery_context_for_training_load(
                                executor,
                                user_uuid,
                                request.tenant_id.as_deref(),
                                sleep_provider_name,
                            )
                            .await
                            {
                                Ok(context) => {
                                    // Add recovery context to analysis
                                    if let Some(obj) = analysis.as_object_mut() {
                                        obj.insert(
                                            "recovery_context".to_owned(),
                                            serde_json::json!({
                                                "sleep_quality_score": context.sleep_quality_score,
                                                "recovery_status": context.recovery_status,
                                                "hrv_available": context.hrv_rmssd.is_some(),
                                                "hrv_rmssd": context.hrv_rmssd,
                                                "sleep_hours": context.sleep_hours,
                                                "sleep_provider": sleep_provider_name,
                                            }),
                                        );
                                    }
                                    Some(context)
                                }
                                Err(err_msg) => {
                                    warn!(
                                        sleep_provider = sleep_provider_name,
                                        error = %err_msg,
                                        "Failed to fetch recovery context, proceeding without"
                                    );
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        // Add provider info
                        if let Some(obj) = analysis.as_object_mut() {
                            obj.insert(
                                "providers_used".to_owned(),
                                serde_json::json!({
                                    "activity_provider": provider_name,
                                    "sleep_provider": sleep_provider,
                                }),
                            );
                        }

                        // Report completion
                        if let Some(reporter) = &request.progress_reporter {
                            reporter.report(
                                100.0,
                                Some(100.0),
                                Some("Training load analysis completed".to_owned()),
                            );
                        }

                        let result = UniversalResponse {
                            success: true,
                            result: Some(analysis),
                            error: None,
                            metadata: Some({
                                let mut map = HashMap::new();
                                map.insert(
                                    "user_id".to_owned(),
                                    serde_json::Value::String(user_uuid.to_string()),
                                );
                                map.insert(
                                    "activity_provider".to_owned(),
                                    serde_json::Value::String(provider_name),
                                );
                                if let Some(sp) = sleep_provider {
                                    map.insert(
                                        "sleep_provider".to_owned(),
                                        serde_json::Value::String(sp.to_owned()),
                                    );
                                }
                                if recovery_context.is_some() {
                                    map.insert(
                                        "recovery_context_included".to_owned(),
                                        serde_json::Value::Bool(true),
                                    );
                                }
                                map
                            }),
                        };

                        // Apply format transformation
                        Ok(apply_format_to_response(
                            result,
                            "training_load",
                            output_format,
                        ))
                    }
                    Err(e) => Ok(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to fetch activities: {e}")),
                        metadata: None,
                    }),
                }
            }
            Err(response) => Ok(response),
        }
    })
}

/// ATL threshold above which we trigger a training load alert notification.
/// Fires when acute training load exceeds 1.5x chronic training load.
#[cfg(feature = "client-notifications")]
const TRAINING_LOAD_ALERT_ATL_RATIO: f64 = 1.5;

/// Fire notification triggers based on training load analysis results.
/// Checks ATL/CTL ratio for training load alerts and TSB for overtraining warnings.
/// All triggers are fire-and-forget — failures are logged but never block the caller.
#[cfg(feature = "client-notifications")]
fn fire_training_load_notifications(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id_str: Option<&str>,
    analysis: &serde_json::Value,
) {
    let Some(service) = &resources.notification_service() else {
        return;
    };
    let Some(tenant_str) = tenant_id_str else {
        return;
    };
    let Ok(tenant_uuid) = tenant_str.parse::<Uuid>() else {
        return;
    };
    let tenant_id = TenantId(tenant_uuid);

    // Extract load metrics from the analysis JSON
    let metrics = &analysis["load_metrics"];
    let atl = metrics["atl"].as_f64().unwrap_or(0.0);
    let ctl = metrics["ctl"].as_f64().unwrap_or(0.0);
    let tsb = metrics["tsb"].as_f64().unwrap_or(0.0);

    // Trigger training load alert when ATL > RATIO * CTL
    if ctl > 0.0 && atl > ctl * TRAINING_LOAD_ALERT_ATL_RATIO {
        notification_triggers::trigger_training_load_alert(service, user_id, tenant_id, atl);
    }

    // Trigger overtraining warning when form drops into the deepest fatigue
    // band relative to the athlete's own chronic load. Athletes with no
    // chronic base band as InsufficientHistory and are never warned on a
    // number that cannot be interpreted.
    if FormBand::from_tsb(tsb, ctl) == FormBand::DeepFatigue {
        notification_triggers::trigger_overtraining_warning(service, user_id, tenant_id);
    }
}
