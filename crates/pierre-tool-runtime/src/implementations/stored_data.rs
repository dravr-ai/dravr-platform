// ABOUTME: The stored health-data tools — sleep, recovery, snapshots, and connected sources
// ABOUTME: Each runs its query body inline against stored rows rather than a provider API

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Stored Health-Data Tools
//!
//! - `GetSleepSessionsTool` — query stored sleep sessions
//! - `GetRecoveryMetricsTool` — query stored recovery and readiness metrics
//! - `GetHealthSnapshotsTool` — query stored health snapshots (body composition, vitals) and
//!   each day's activity totals (steps, energy, intensity minutes)
//! - `ListDataSourcesTool` — list connected data sources (devices and providers)
//!
//! These read rows the sync pipeline already persisted, so each executes its
//! query body inline instead of bridging into a provider-API handler. They
//! share one date-range + format argument shape, built by
//! [`date_range_properties`] and parsed by [`parse_date_range`] /
//! [`apply_format`].
//!
//! The sleep, recovery, snapshot and activity rows are read through
//! `pierre_services::stored_health`, which applies each row's provider terms
//! (carnet#771) and then merges them per night or per day (carnet#1281).

use crate::annotations::{annotations, Effect, Reach};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{
    DataSource, Merged, StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, TenantId,
};
use pierre_core::untrusted::fence_athlete_text;
use serde::Serialize;
use serde_json::{json, Value};

use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, format_property, object_schema, ok_typed, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::data_helpers::parse_output_format;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_services::stored_health::{self, SourceFilter};
use pierre_tools_core::ToolResult;

// ============================================================================
// Helper - argument parsing and schema builders for stored health-data tools
// ============================================================================

/// Default lookback window when no explicit date range is supplied.
const DEFAULT_LOOKBACK_DAYS: i64 = 30;

/// Widest span one stored-data read serves. Population is one row per night
/// per user, so a year bounds the result set; the underlying queries carry no
/// SQL LIMIT, which made an unclamped caller range the only thing standing
/// between a read and the whole table.
const MAX_RANGE_DAYS: i64 = 366;

/// Parse `start`/`end` RFC3339 args, defaulting to (end - 30d, now).
///
/// An omitted `start` is measured back from `end`, so a caller that names
/// only the end of a past window gets the 30 days before it rather than a
/// start after its own end.
///
/// An inverted range is refused; a span wider than [`MAX_RANGE_DAYS`] is
/// clipped to the most recent year (the payload echoes the effective
/// `start`/`end`, so the clip is visible to the caller). `pub` so the clamp
/// decisions are exercisable by the integration test suite.
///
/// # Errors
///
/// Returns `invalid_input` when `start` is after `end`.
pub fn parse_date_range(args: &Value) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let end = args
        .get("end")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));
    let mut start = args
        .get("start")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(
            || end - Duration::days(DEFAULT_LOOKBACK_DAYS),
            |dt| dt.with_timezone(&Utc),
        );
    if start > end {
        return Err(AppError::invalid_input(format!(
            "start ({start}) is after end ({end})"
        )));
    }
    if end - start > Duration::days(MAX_RANGE_DAYS) {
        start = end - Duration::days(MAX_RANGE_DAYS);
    }
    Ok((start, end))
}

/// When each of the athlete's connected sources last synced `data_type` (the
/// health sync's cursor name: `sleep`, `recovery`, `health`), by provider.
/// A source whose cursor cannot be read is reported as never synced.
async fn last_synced(
    context: &ToolExecutionContext,
    tenant_id: &TenantId,
    data_type: &str,
) -> BTreeMap<String, Option<String>> {
    let repos = context.resources.repos();
    let user = context.user_id.to_string();
    let sources = repos
        .data_sources
        .list_data_sources(context.user_id, tenant_id)
        .await
        .unwrap_or_default();
    let mut synced = BTreeMap::new();
    for source in sources {
        if synced.contains_key(&source.provider) {
            continue;
        }
        let at = repos
            .sync_cursors
            .get_sync_cursor(&user, tenant_id, &source.provider, data_type)
            .await
            .ok()
            .flatten()
            .and_then(|cursor| cursor.last_sync_at)
            .map(|at| at.to_rfc3339());
        synced.insert(source.provider, at);
    }
    synced
}

/// Build the standard date-range + format property set used by stored
/// health-data queries (sleep, recovery, snapshots), whose handlers read
/// `start`, `end` and `format` from `request.parameters`.
fn date_range_properties() -> BTreeMap<String, PropertySchema> {
    let mut properties = BTreeMap::new();
    properties.insert(
        "start".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(format!(
                "RFC3339 start; defaults to {DEFAULT_LOOKBACK_DAYS} days before `end`; a \
                 range longer than {MAX_RANGE_DAYS} days keeps its most recent {MAX_RANGE_DAYS}."
            )),
            ..Default::default()
        },
    );
    properties.insert(
        "end".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some("RFC3339 end; defaults to now.".to_owned()),
            ..Default::default()
        },
    );
    properties.insert("format".to_owned(), format_property());
    properties
}

/// The window a stored-data read covered.
///
/// Echoed back rather than left implicit: the caller may omit the range, in
/// which case the server picks the last thirty days, and a client that assumed
/// its own default would mislabel the data it got.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DateRange {
    /// RFC 3339 start of the window, inclusive.
    pub start: String,
    /// RFC 3339 end of the window.
    pub end: String,
}

/// What `get_sleep_sessions` answers with.
///
/// The sessions are dravr-equilibre's own `StoredSleepSession`, forwarded
/// rather than projected. That crate derives `JsonSchema` for exactly this
/// reason, so the declared schema is the stored shape and cannot drift from
/// it by a projection someone forgot to update. Each is one sleep merged
/// across the athlete's sources, carrying `sources` and the metrics it took
/// from a source other than its primary.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SleepSessionsResult {
    /// How many sleeps the window held, after merging sources.
    pub count: usize,
    /// One entry per sleep, merged across sources.
    pub sessions: Vec<Merged<StoredSleepSession>>,
    /// The attribution these readings must be shown and quoted with: the
    /// athlete's Garmin device (`Garmin Forerunner 965`) when intervals.icu
    /// wellness is among them and the athlete's intervals.icu activities were
    /// recorded on it (carnet#521); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// The window that was read.
    pub range: DateRange,
    /// When each connected source last synced this data, RFC 3339 by
    /// provider (`null` when it never has), so an empty or short window can
    /// be told apart from a source that stopped syncing.
    pub last_synced_at: BTreeMap<String, Option<String>>,
}

/// What `get_recovery_metrics` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecoveryMetricsResult {
    /// How many days the window held, after merging sources.
    pub count: usize,
    /// One reading per day, merged across sources.
    pub metrics: Vec<Merged<StoredRecoveryMetrics>>,
    /// The attribution these readings must be shown and quoted with: the
    /// athlete's Garmin device (`Garmin Forerunner 965`) when intervals.icu
    /// wellness is among them and the athlete's intervals.icu activities were
    /// recorded on it (carnet#521); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// The window that was read.
    pub range: DateRange,
    /// When each connected source last synced this data, RFC 3339 by
    /// provider (`null` when it never has), so an empty or short window can
    /// be told apart from a source that stopped syncing.
    pub last_synced_at: BTreeMap<String, Option<String>>,
}

/// What `get_health_snapshots` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HealthSnapshotsResult {
    /// How many days the window held, after merging sources.
    pub count: usize,
    /// One snapshot per day, merged across sources.
    pub snapshots: Vec<Merged<StoredHealthMetrics>>,
    /// Each day's steps, energy and intensity minutes, merged across
    /// sources.
    pub daily_activity: Vec<Merged<StoredDailyActivity>>,
    /// The attribution these readings must be shown and quoted with: the
    /// athlete's Garmin device (`Garmin Forerunner 965`) when intervals.icu
    /// wellness is among them and the athlete's intervals.icu activities were
    /// recorded on it (carnet#521); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// The window that was read.
    pub range: DateRange,
    /// When each connected source last synced this data, RFC 3339 by
    /// provider (`null` when it never has), so an empty or short window can
    /// be told apart from a source that stopped syncing.
    pub last_synced_at: BTreeMap<String, Option<String>>,
}

/// What `list_data_sources` answers with.
///
/// No range: the sources an athlete has connected are current state, not a
/// window over history.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DataSourcesResult {
    /// How many sources are on file.
    pub count: usize,
    /// The sources, as stored.
    pub sources: Vec<DataSource>,
}

// ============================================================================
// GetSleepSessionsTool - Query stored sleep sessions
// ============================================================================

/// Tool for querying stored sleep sessions from the database.
#[derive(Debug)]
pub struct GetSleepSessionsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetSleepSessionsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<SleepSessionsResult>>(tool_definition(
            "get_sleep_sessions",
            "Get stored sleep sessions from the database",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
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
            let (start, end) = parse_date_range(&args)?;

            match stored_health::merged_sleep(
                context.resources.repos(),
                context.resources.provider_registry().as_ref(),
                context.user_id,
                &tenant_id,
                (start, end),
                SourceFilter::All,
            )
            .await
            {
                Ok(read) => {
                    let sessions = read.records;
                    let last_synced_at = last_synced(&context, &tenant_id, "sleep").await;
                    let payload = SleepSessionsResult {
                        count: sessions.len(),
                        sessions,
                        attribution: read.attribution,
                        last_synced_at,
                        range: DateRange {
                            start: start.to_rfc3339(),
                            end: end.to_rfc3339(),
                        },
                    };
                    ok_typed("get_sleep_sessions", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch sleep sessions: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Longest athlete note, in characters, a recovery reading carries to a model.
const MAX_ATHLETE_NOTE_CHARS: usize = 600;

/// Fence each day's athlete note as untrusted text before a model reads it.
///
/// The note is whatever the athlete typed into their source's wellness log;
/// fenced, it reads as quoted data rather than as instructions, and an empty
/// note drops out.
fn fence_athlete_notes(
    days: Vec<Merged<StoredRecoveryMetrics>>,
) -> Vec<Merged<StoredRecoveryMetrics>> {
    days.into_iter()
        .map(|mut day| {
            day.record.athlete_note = day
                .record
                .athlete_note
                .as_deref()
                .and_then(|note| fence_athlete_text(note, MAX_ATHLETE_NOTE_CHARS));
            day
        })
        .collect()
}

// ============================================================================
// GetRecoveryMetricsTool - Query stored recovery and readiness metrics
// ============================================================================

/// Tool for querying stored recovery and readiness data from the database.
#[derive(Debug)]
pub struct GetRecoveryMetricsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetRecoveryMetricsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<RecoveryMetricsResult>>(tool_definition(
            "get_recovery_metrics",
            "Get stored recovery and readiness metrics",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
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
            let (start, end) = parse_date_range(&args)?;

            match stored_health::merged_recovery(
                context.resources.repos(),
                context.resources.provider_registry().as_ref(),
                context.user_id,
                &tenant_id,
                (start, end),
                SourceFilter::All,
            )
            .await
            {
                Ok(read) => {
                    let metrics = fence_athlete_notes(read.records);
                    let last_synced_at = last_synced(&context, &tenant_id, "recovery").await;
                    let payload = RecoveryMetricsResult {
                        count: metrics.len(),
                        metrics,
                        attribution: read.attribution,
                        last_synced_at,
                        range: DateRange {
                            start: start.to_rfc3339(),
                            end: end.to_rfc3339(),
                        },
                    };
                    ok_typed("get_recovery_metrics", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch recovery metrics: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetHealthSnapshotsTool - Query stored health snapshots
// ============================================================================

/// Tool for querying stored body composition and vitals snapshots.
#[derive(Debug)]
pub struct GetHealthSnapshotsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetHealthSnapshotsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<HealthSnapshotsResult>>(tool_definition(
            "get_health_snapshots",
            "Get stored health snapshots (body composition, vitals) and daily steps, energy, intensity minutes",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
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
            let (start, end) = parse_date_range(&args)?;

            let repos = context.resources.repos();
            let terms = context.resources.provider_registry();
            let read = async {
                let body = stored_health::merged_body(
                    repos,
                    terms.as_ref(),
                    context.user_id,
                    &tenant_id,
                    (start, end),
                    SourceFilter::All,
                )
                .await?;
                let activity = stored_health::merged_daily_activity(
                    repos,
                    terms.as_ref(),
                    context.user_id,
                    &tenant_id,
                    (start, end),
                    SourceFilter::All,
                )
                .await?;
                AppResult::Ok((body, activity))
            };
            match read.await {
                Ok((body, activity)) => {
                    let snapshots = body.records;
                    let last_synced_at = last_synced(&context, &tenant_id, "health").await;
                    let payload = HealthSnapshotsResult {
                        count: snapshots.len(),
                        snapshots,
                        daily_activity: activity.records,
                        attribution: body.attribution.or(activity.attribution),
                        last_synced_at,
                        range: DateRange {
                            start: start.to_rfc3339(),
                            end: end.to_rfc3339(),
                        },
                    };
                    ok_typed("get_health_snapshots", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch health snapshots: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListDataSourcesTool - List connected devices and providers
// ============================================================================

/// Tool for listing connected data sources (devices and providers) for the user.
#[derive(Debug)]
pub struct ListDataSourcesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListDataSourcesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert("format".to_owned(), format_property());
        let schema = object_schema(properties, None);

        answers_with::<Formatted<DataSourcesResult>>(tool_definition(
            "list_data_sources",
            "List connected data sources (devices and providers)",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
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

            match context
                .resources
                .repos()
                .data_sources
                .list_data_sources(context.user_id, &tenant_id)
                .await
            {
                Ok(sources) => {
                    let payload = DataSourcesResult {
                        count: sources.len(),
                        sources,
                    };
                    ok_typed("list_data_sources", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to list data sources: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
//
// Provider-synced health records: the same third-party-scrape provenance as
// GetActivities/GetAthlete (Whoop/Garmin/Terra), carrying
// provider-controlled free-text fields (device/source names, data_source_id —
// cf. the garmin data_source_id blob leak). A taint SOURCE, so a later
// consequential sink in the same turn is gated.
crate::declare_security!(GetSleepSessionsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetRecoveryMetricsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetHealthSnapshotsTool => UNTRUSTED_OUTPUT);
// Surfaces provider/source names synced from third parties as free text.
crate::declare_security!(ListDataSourcesTool => UNTRUSTED_OUTPUT);
