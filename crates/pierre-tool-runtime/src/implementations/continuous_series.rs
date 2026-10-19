// ABOUTME: get_heart_rate_series — the athlete's stored intraday heart rate and HRV, as points or windowed aggregates
// ABOUTME: Reads riviere's TimeSeriesStore through pierre_services::continuous_series over the athlete's own sources

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `get_heart_rate_series`
//!
//! The continuous series the sync stores, read back (carnet#999): every
//! heart-rate or HRV sample a source recorded in a time range, or one value
//! per window when `window_minutes` is given. The daily readings —
//! resting heart rate, overnight HRV — stay with `get_recovery_metrics`;
//! this tool is the intraday series under them.
//!
//! A sample read is bounded twice: by span (`MAX_POINTS_RANGE_HOURS`) and
//! by count (`MAX_POINTS`), since a source can sample once a second. A
//! windowed read may span `MAX_WINDOWED_RANGE_DAYS` in at most
//! `MAX_WINDOWS` windows. A wider span keeps its most recent part, and the
//! payload echoes the window that was read.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dravr_riviere::aggregation::window_count;
use dravr_riviere::{Aggregation, SeriesType};
use pierre_core::models::TenantId;
use serde::Serialize;
use serde_json::{json, Value};

use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, object_schema_with_format, ok_typed, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::data_helpers::{parse_output_format, read_only_annotations};
use crate::implementations::stored_data::DateRange;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::PropertySchema;
use pierre_services::continuous_series::{
    continuous_metric, continuous_series, SeriesRead, SourceSeries, CONTINUOUS_METRICS,
};
use pierre_tools_core::ToolResult;

/// Span read when the caller names no `start`.
const DEFAULT_LOOKBACK_HOURS: i64 = 24;

/// Widest span a sample read covers.
const MAX_POINTS_RANGE_HOURS: i64 = 48;

/// Most samples a read returns across sources; past it the caller is told to
/// window the read instead.
const MAX_POINTS: usize = 5_000;

/// Widest span a windowed read covers.
const MAX_WINDOWED_RANGE_DAYS: i64 = 31;

/// Most windows one windowed read folds the span into.
const MAX_WINDOWS: i64 = 1_000;

/// The aggregations a windowed read accepts, by the name the caller passes.
/// `sum` is left out: a sum of heart-rate samples measures nothing.
const AGGREGATIONS: [(&str, Aggregation); 6] = [
    ("avg", Aggregation::Avg),
    ("min", Aggregation::Min),
    ("max", Aggregation::Max),
    ("count", Aggregation::Count),
    ("first", Aggregation::First),
    ("last", Aggregation::Last),
];

/// The window a windowed read folded samples into.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SeriesWindow {
    /// Width of each window, in minutes.
    pub minutes: i64,
    /// How each window's samples were folded (`avg`, `min`, `max`, `count`,
    /// `first`, `last`).
    pub aggregation: String,
}

/// What `get_heart_rate_series` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HeartRateSeriesResult {
    /// The series read (`heart_rate`, `hrv_rmssd`, `hrv_sdnn`).
    pub metric: String,
    /// The unit of every value (`bpm`, `ms`).
    pub unit: String,
    /// The windowing applied; absent for a sample read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<SeriesWindow>,
    /// How many samples, or windows, the sources hold in the range.
    pub count: usize,
    /// One series per data source holding data in the range; sources are
    /// never merged.
    pub sources: Vec<SourceSeries>,
    /// The attribution these readings must be shown and quoted with: the
    /// athlete's Garmin device (`Garmin Forerunner 965`) when an intervals.icu
    /// series is among them and the athlete's intervals.icu activities were
    /// recorded on it (carnet#521); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// The window that was read.
    pub range: DateRange,
}

/// The series named by `metric`, defaulting to heart rate.
fn parse_metric(args: &Value) -> AppResult<SeriesType> {
    let Some(name) = args.get("metric").and_then(Value::as_str) else {
        return Ok(SeriesType::HeartRate);
    };
    continuous_metric(name).ok_or_else(|| {
        AppError::invalid_input(format!(
            "unknown metric '{name}'; expected one of: {}",
            metric_names().join(", ")
        ))
    })
}

fn metric_names() -> Vec<&'static str> {
    CONTINUOUS_METRICS.iter().map(|s| s.as_str()).collect()
}

fn aggregation_names() -> Vec<&'static str> {
    AGGREGATIONS.iter().map(|(name, _)| *name).collect()
}

/// The windowing `window_minutes` and `aggregation` ask for; `None` reads
/// every sample.
fn parse_window(args: &Value) -> AppResult<Option<(i64, &'static str, Aggregation)>> {
    let Some(raw) = args.get("window_minutes").filter(|v| !v.is_null()) else {
        if args.get("aggregation").is_some_and(|v| !v.is_null()) {
            return Err(AppError::invalid_input(
                "aggregation applies only to a windowed read; pass window_minutes too",
            ));
        }
        return Ok(None);
    };
    let minutes = raw
        .as_i64()
        .filter(|m| *m >= 1)
        .ok_or_else(|| AppError::invalid_input("window_minutes must be a whole number >= 1"))?;
    let name = args
        .get("aggregation")
        .and_then(Value::as_str)
        .unwrap_or("avg");
    let (name, aggregation) = AGGREGATIONS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .copied()
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "unknown aggregation '{name}'; expected one of: {}",
                aggregation_names().join(", ")
            ))
        })?;
    Ok(Some((minutes, name, aggregation)))
}

/// Parse an RFC 3339 argument, refusing one that does not parse rather than
/// silently reading a default window in its place.
fn parse_instant(args: &Value, key: &str) -> AppResult<Option<DateTime<Utc>>> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| AppError::invalid_input(format!("{key} is not RFC 3339: {e}")))
        })
        .transpose()
}

/// The half-open range a read covers: `end` defaults to now, `start` to
/// [`DEFAULT_LOOKBACK_HOURS`] before `end`, and a span wider than
/// `max_span` keeps its most recent `max_span`.
///
/// # Errors
///
/// Returns `invalid_input` when an instant does not parse or `start` is not
/// before `end`.
fn parse_series_range(
    args: &Value,
    max_span: Duration,
) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let end = parse_instant(args, "end")?.unwrap_or_else(Utc::now);
    let mut start =
        parse_instant(args, "start")?.unwrap_or(end - Duration::hours(DEFAULT_LOOKBACK_HOURS));
    if start >= end {
        return Err(AppError::invalid_input(format!(
            "start ({start}) must be before end ({end})"
        )));
    }
    if end - start > max_span {
        start = end - max_span;
    }
    Ok((start, end))
}

fn string_property(description: String) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description),
        ..Default::default()
    }
}

fn input_properties() -> HashMap<String, PropertySchema> {
    let mut properties = HashMap::new();
    properties.insert(
        "metric".to_owned(),
        string_property(format!(
            "The series to read; defaults to heart_rate. One of: {}.",
            metric_names().join(", ")
        )),
    );
    properties.insert(
        "start".to_owned(),
        string_property(format!(
            "Start of the range as RFC3339 timestamp, inclusive. Defaults to \
             {DEFAULT_LOOKBACK_HOURS} hours before `end`. A sample read spans at most \
             {MAX_POINTS_RANGE_HOURS} hours and a windowed read {MAX_WINDOWED_RANGE_DAYS} \
             days; a wider range keeps its most recent part."
        )),
    );
    properties.insert(
        "end".to_owned(),
        string_property(
            "End of the range as RFC3339 timestamp, exclusive. Defaults to now.".to_owned(),
        ),
    );
    properties.insert(
        "window_minutes".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some(format!(
                "Fold the samples into one value per window of this many minutes \
                 (at most {MAX_WINDOWS} windows over the range). Omit to read every sample."
            )),
            ..Default::default()
        },
    );
    properties.insert(
        "aggregation".to_owned(),
        string_property(format!(
            "How a window's samples fold into its value, with `window_minutes`; defaults \
             to avg. One of: {}.",
            aggregation_names().join(", ")
        )),
    );
    properties
}

/// Refuse a windowing that folds the range into more than [`MAX_WINDOWS`].
fn check_window_count(
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    window_secs: i64,
) -> AppResult<()> {
    let windows = window_count(start, end, window_secs)
        .map_err(|e| AppError::invalid_input(format!("window_minutes: {e}")))?;
    if windows > MAX_WINDOWS {
        return Err(AppError::invalid_input(format!(
            "window_minutes folds this range into {windows} windows, more than \
             {MAX_WINDOWS}; widen the window or narrow the range"
        )));
    }
    Ok(())
}

/// Refuse a sample read past [`MAX_POINTS`], pointing the caller at a
/// windowed read.
fn check_point_count(count: usize) -> AppResult<()> {
    if count > MAX_POINTS {
        return Err(AppError::invalid_input(format!(
            "the range holds {count} samples, more than {MAX_POINTS}; pass window_minutes \
             to aggregate them or narrow the range"
        )));
    }
    Ok(())
}

// ============================================================================
// GetHeartRateSeriesTool
// ============================================================================

/// Tool reading the athlete's stored continuous heart-rate and HRV series.
pub struct GetHeartRateSeriesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetHeartRateSeriesTool {
    fn definition(&self) -> Tool {
        answers_with::<Formatted<HeartRateSeriesResult>>(tool_definition(
            "get_heart_rate_series",
            "Get the athlete's stored intraday heart rate or HRV samples over a time range, \
             one series per connected source, or one aggregated value per window when \
             `window_minutes` is given. For daily resting heart rate and overnight HRV use \
             get_recovery_metrics.",
            object_schema_with_format(input_properties(), None),
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let series = parse_metric(&args)?;
            let window = parse_window(&args)?;
            let max_span = if window.is_some() {
                Duration::days(MAX_WINDOWED_RANGE_DAYS)
            } else {
                Duration::hours(MAX_POINTS_RANGE_HOURS)
            };
            let range = parse_series_range(&args, max_span)?;
            let read = match window {
                Some((minutes, _, aggregation)) => {
                    // A window wider than the range is one window over all of it.
                    let span_secs = (range.1 - range.0).num_seconds();
                    let window_secs = minutes.saturating_mul(60).min(span_secs);
                    check_window_count(range, window_secs)?;
                    SeriesRead::Windows {
                        window_secs,
                        aggregation,
                    }
                }
                None => SeriesRead::Points,
            };

            let served = match continuous_series(
                context.resources.repos(),
                context.resources.provider_registry().as_ref(),
                context.user_id,
                &tenant_id,
                series,
                range,
                read,
            )
            .await
            {
                Ok(served) => served,
                Err(e) => {
                    return Ok(ToolResult::error(json!({
                        "error": format!("Failed to read the {} series: {e}", series.as_str()),
                    })));
                }
            };
            let count = served.records.iter().map(SourceSeries::len).sum();
            if window.is_none() {
                check_point_count(count)?;
            }
            let payload = HeartRateSeriesResult {
                metric: series.as_str().to_owned(),
                unit: series.definition().unit.to_owned(),
                window: window.map(|(minutes, name, _)| SeriesWindow {
                    minutes,
                    aggregation: name.to_owned(),
                }),
                count,
                sources: served.records,
                attribution: served.attribution,
                range: DateRange {
                    start: range.0.to_rfc3339(),
                    end: range.1.to_rfc3339(),
                },
            };
            ok_typed("get_heart_rate_series", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// Provider-synced samples, with the provenance of the other stored health
// reads: a taint SOURCE, so a later consequential sink in the turn is gated.
crate::declare_security!(GetHeartRateSeriesTool => UNTRUSTED_OUTPUT);
