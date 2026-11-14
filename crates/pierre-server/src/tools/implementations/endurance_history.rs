// ABOUTME: Endurance Phase 2 MCP tools — compute_training_history + get_training_history
// ABOUTME: On-demand backfill + read of the daily CTL/ATL/TSB training-history rollup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_tool_runtime::annotations::{annotations, Effect, Reach};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, NaiveTime};
use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::coverage::{Completeness, Coverage, CoverageWindow, IncompleteReason};
use pierre_core::models::{
    DailyTrainingState, FormBand, FormInterpretation, FormReading, TenantId,
};
use pierre_fitness_compute::training_history_compute::MAX_BACKFILL_DAYS;
use pierre_providers::{ai_scope, backend_resolver};
use pierre_services::training_history_read::user_timezone;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tool_runtime::context::{scoped_read_subject, ToolExecutionContext};
use pierre_tool_runtime::conversions::{
    answers_with, ok_typed, task_capable, tool_definition, tool_result_to_response,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::security::RuntimeTool;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, default_window, history_rows_for_model, history_walk_pending,
    read_history_from_cache, TrainingHistoryComputed, DEFAULT_BACKFILL_DAYS,
};
use pierre_tools_core::ToolResult;

/// One day of training state, with its form reading attached.
///
/// The stored row is flattened in rather than nested: this is what
/// `get_training_history` has always sent, and an agent reading `ctl` should
/// not have to know whether it sits under a `state` key.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TrainingHistoryDay {
    /// The persisted daily rollup.
    #[serde(flatten)]
    pub state: DailyTrainingState,
    /// Form as a percentage of this athlete's own CTL — the day's `tsb` over
    /// its `form_ctl`, both from the end of the day before — rounded. Null when
    /// there is no chronic base to normalise against — form cannot be judged
    /// at all in that case, and a zero would read as "balanced".
    pub tsb_pct_of_ctl: Option<f64>,
    /// The band that percentage falls in. Never an injury verdict.
    pub form_band: FormBand,
}

/// What `get_training_history` answers with.
///
/// Every row carries its own form reading, and the payload carries the method
/// that produced it. This is the tool the endurance agent prompt calls first;
/// shipping bare ctl/atl/tsb floats is how a raw `-77` reached an athlete as a
/// diagnosis nobody could explain (registre#199). Beside the rows ride the
/// evidence a "why" traces them to: the services whose sessions they are
/// computed from, and the run of deep-fatigue days a reading closes
/// (carnet#1387).
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TrainingHistoryResult {
    /// First day of the window, as `YYYY-MM-DD`.
    pub from: String,
    /// Last day, inclusive.
    pub to: String,
    /// One entry per day that has a rollup.
    pub days: Vec<TrainingHistoryDay>,
    /// Each service a session in the window came from, with the last day it
    /// delivered one there. Empty when no session in the window is readable.
    pub sources: Vec<DeliveringService>,
    /// The consecutive days ending on `to` whose form fell in deep fatigue,
    /// oldest first. Empty when `to` has no row or its form is not deep
    /// fatigue. A run describes accumulated fatigue relative to fitness, not
    /// overtraining.
    pub deep_fatigue_mornings: Vec<DeepFatigueMorning>,
    /// How much of the window `days` answers: `complete` when the rows
    /// answer the whole ask; otherwise why not (the stored history does not
    /// reach far enough to warm the earlier days, or nothing is stored), the
    /// first day the rows stand behind, the history walk still owed for the
    /// missing depth (`pending`), and the `note` saying what may be cited.
    pub coverage: Coverage,
    /// What the numbers mean and how they were computed.
    pub interpretation: FormInterpretation,
}

/// A service the rows' sessions came from.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeliveringService {
    /// The service's key, as `connect_provider` takes it.
    pub provider: String,
    /// The service as the athlete knows it.
    pub brand: String,
    /// The athlete's civil date of the last session it delivered in the
    /// window, as `YYYY-MM-DD`.
    pub last_activity: String,
}

/// One day of a deep-fatigue run.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeepFatigueMorning {
    /// The day, as `YYYY-MM-DD`.
    pub date: String,
    /// Form that day as a percentage of the athlete's own CTL, rounded.
    pub tsb_pct_of_ctl: Option<f64>,
}

/// Sessions read for [`delivering_services`]: a window of daily rows holds far
/// fewer, and only the newest of each service counts.
const SOURCE_READ_LIMIT: i64 = 500;

/// The services the window's sessions came from, newest delivery first, as
/// the call may read them: a provider whose terms or the athlete's consent
/// keep its sessions from the reader is not named.
async fn delivering_services(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DeliveringService>> {
    let zone = resolve_zone(user_timezone(resources.repos(), user_id).await?.as_deref());
    let start = (from - Duration::days(1))
        .and_time(NaiveTime::MIN)
        .and_utc();
    let end = (to + Duration::days(2)).and_time(NaiveTime::MIN).and_utc();
    let stored = resources
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, None, start, end, SOURCE_READ_LIMIT)
        .await?;
    let readable = ai_scope::filter_activities(resources.provider_registry().as_ref(), stored);
    let mut last: BTreeMap<String, NaiveDate> = BTreeMap::new();
    for activity in &readable {
        let day = local_date(activity.start_date(), zone);
        if day < from || day > to {
            continue;
        }
        let provider = backend_resolver::user_facing_name(activity.provider()).to_owned();
        last.entry(provider)
            .and_modify(|latest| *latest = (*latest).max(day))
            .or_insert(day);
    }
    let mut sources: Vec<DeliveringService> = last
        .into_iter()
        .map(|(provider, day)| DeliveringService {
            brand: resources
                .provider_registry()
                .get_display_name(&provider)
                .unwrap_or(provider.as_str())
                .to_owned(),
            last_activity: day.format("%Y-%m-%d").to_string(),
            provider,
        })
        .collect();
    sources.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    Ok(sources)
}

/// The consecutive days ending on `to` whose form fell in deep fatigue,
/// oldest first; empty unless the row for `to` is one of them.
fn deep_fatigue_run(days: &[TrainingHistoryDay], to: NaiveDate) -> Vec<DeepFatigueMorning> {
    let mut run: Vec<DeepFatigueMorning> = Vec::new();
    let mut expected = to;
    for day in days.iter().rev() {
        if day.state.date != expected || day.form_band != FormBand::DeepFatigue {
            break;
        }
        run.push(DeepFatigueMorning {
            date: day.state.date.format("%Y-%m-%d").to_string(),
            tsb_pct_of_ctl: day.tsb_pct_of_ctl,
        });
        expected = day.state.date - Duration::days(1);
    }
    run.reverse();
    run
}

/// What `compute_training_history` answers with.
///
/// A write tool that reports its own scope. The window is echoed because the
/// caller may have passed none and taken the default, and `rows_upserted` is
/// how an agent knows whether the recompute actually had days to work with —
/// zero is a valid answer for a window the athlete did not train in.
///
/// `from` is the window actually computed, which is later than `requested_from`
/// when stored history could not warm the whole ask. Both are reported because
/// `ctl`/`atl`/`tsb` are plain floats seeded at zero: a day computed without its
/// CTL warm-up behind it carries a chronic load that is wrong low and looks
/// exactly like a real one, so the agent has to be told which days exist rather
/// than inferring it from a row count.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ComputeTrainingHistoryResult {
    /// First day actually recomputed, as `YYYY-MM-DD`.
    pub from: String,
    /// Last day of the window, inclusive.
    pub to: String,
    /// How many daily rows were written or overwritten.
    pub rows_upserted: usize,
    /// First day the caller asked for, as `YYYY-MM-DD`. Differs from `from`
    /// when stored history was too shallow to warm the whole window.
    pub requested_from: String,
    /// Whether every requested day was backed by enough stored history, the
    /// history walk still owed when not (`pending`), and the `note` saying
    /// what the agent may and may not claim.
    pub coverage: Coverage,
    /// Whether a background capture of the missing history was started.
    pub capture_requested: bool,
}

impl From<TrainingHistoryComputed> for ComputeTrainingHistoryResult {
    fn from(c: TrainingHistoryComputed) -> Self {
        Self {
            from: c.from.format("%Y-%m-%d").to_string(),
            to: c.to.format("%Y-%m-%d").to_string(),
            rows_upserted: c.rows_upserted,
            requested_from: c.requested_from.format("%Y-%m-%d").to_string(),
            coverage: c.coverage,
            capture_requested: c.capture_requested,
        }
    }
}

/// `coverage` with the history walk still owed for it, when it falls short.
async fn with_pending(
    resources: &Arc<dyn ToolRuntime>,
    (tenant_id, user_id): (TenantId, Uuid),
    coverage: Coverage,
) -> AppResult<Coverage> {
    if coverage.is_complete() {
        return Ok(coverage);
    }
    Ok(Coverage {
        pending: history_walk_pending(resources, tenant_id, user_id).await?,
        ..coverage
    })
}

fn require_tenant(context: &ToolExecutionContext) -> AppResult<TenantId> {
    context.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthInvalid,
            "Endurance history tools require an active tenant context",
        )
    })
}

fn parse_date(args: &Value, key: &str) -> AppResult<Option<NaiveDate>> {
    let Some(raw) = args.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(Some)
        .map_err(|e| AppError::invalid_input(format!("{key} must be YYYY-MM-DD: {e}")))
}

/// Resolve `[from, to]` from the tool arguments, defaulting to the athlete's
/// own civil day.
///
/// `to` defaults to today *where the athlete is*, not where the server is: the
/// rollup buckets each activity on their civil date, so a UTC-dated bound put
/// their current local day past `to` for every zone ahead of UTC and dropped
/// the session they had just finished (registre#260).
async fn resolve_window(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    args: &Value,
) -> AppResult<(NaiveDate, NaiveDate)> {
    let (_, athlete_today) = default_window(resources, user_id).await?;
    let to = parse_date(args, "to")?.unwrap_or(athlete_today);
    let from =
        parse_date(args, "from")?.unwrap_or_else(|| to - Duration::days(DEFAULT_BACKFILL_DAYS));
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }
    Ok((from, to))
}

fn date_range_schema() -> JsonSchema {
    let mut properties = HashMap::new();
    properties.insert(
        "from".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Inclusive start date, YYYY-MM-DD; defaults to 90 days before `to`.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "to".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Inclusive end date, YYYY-MM-DD; defaults to today in the athlete's timezone."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    JsonSchema {
        schema_type: "object".to_owned(),
        properties: Some(properties.into_iter().collect()),
        required: Some(Vec::new()),
        ..Default::default()
    }
}

/// `compute_training_history` — on-demand backfill of the daily rollup.
#[derive(Debug)]
pub struct ComputeTrainingHistoryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ComputeTrainingHistoryTool {
    fn definition(&self) -> Tool {
        let schema = date_range_schema();
        answers_with::<ComputeTrainingHistoryResult>(task_capable(tool_definition(
            "compute_training_history",
            "Compute and store the Endurance daily training-state rows that \
             get_training_history reads — CTL, ATL, TSB (Coggan), acute:chronic load \
             balance, monotony + strain (Foster), ramp rate and daily TSS load. Use it the \
             first time, after a long gap in syncs, or when the user asks for a specific \
             historical window.",
            schema,
            annotations(
                "Compute training history",
                Effect::IdempotentWrite,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = require_tenant(&context)?;
            let user_id = context.user_id;
            let (from, to) = resolve_window(&context.resources, user_id, &args).await?;
            let computed =
                compute_and_persist_history(&context.resources, tenant_id, user_id, from, to)
                    .await?;
            ok_typed(
                "compute_training_history",
                ComputeTrainingHistoryResult::from(computed),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `get_training_history` — read the persisted daily rollup.
///
/// A training-class read: for a coach's read of a coached athlete the
/// dispatch chokepoint builds the call's context as the athlete, so the
/// rows read are theirs (carnet#1387).
#[derive(Debug)]
pub struct GetTrainingHistoryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetTrainingHistoryTool {
    fn definition(&self) -> Tool {
        let schema = date_range_schema();
        answers_with::<TrainingHistoryResult>(tool_definition(
            "get_training_history",
            "Fetch the stored Endurance daily training-state rows, chronological: \
             CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load. Use this BEFORE \
             prescribing new load, so every numeric claim cites these framework values \
             (Coggan / Foster). Interpret TSB relative to CTL (form as % of fitness), and \
             report `acwr` strictly as the magnitude of 7-day load against the 28-day \
             baseline — a descriptive ratio, not a predictor of harm. Sessions are scored \
             against the FTP and threshold heart rate saved with set_physiology; a save that \
             changes either recomputes the rows.",
            schema,
            annotations(
                "Get training history",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = require_tenant(&context)?;
            let user_id = context.user_id;
            let (from, to) = resolve_window(&context.resources, user_id, &args).await?;
            let mut rows =
                history_rows_for_model(&context.resources, tenant_id, user_id, from, to).await?;
            // An athlete's rollup is computed when their own surfaces or a
            // capture compute it, so a coach reading it can come first: rows
            // that stop short of `to` are computed from the athlete's stored
            // sessions, under their tenant, before they are read (carnet#1387).
            if scoped_read_subject().is_some() && rows.last().is_none_or(|row| row.date < to) {
                compute_and_persist_history(&context.resources, tenant_id, user_id, from, to)
                    .await?;
                rows = history_rows_for_model(&context.resources, tenant_id, user_id, from, to)
                    .await?;
            }
            // No persisted row, or rows that start after `from`: the read a
            // page makes says why, and answers from the stored sessions when
            // they do warm the window. The rollup holds only the days some
            // compute vouched for, so rows that start late cannot tell a
            // history too short to warm the window from a window never
            // computed; the stored sessions can, and the rows and their
            // coverage then come from that one read, unless it warms no day
            // the rollup does. It writes no row, so a read-only grant stays
            // one. Without it, an athlete with too short a history read
            // `days: []` beside a fitness score computed from the same
            // sessions. A window short of history then reads the walk
            // owed for it, and asks one where none ever ran: a background
            // capture of the athlete's account, as `get_activities` asks for a
            // deep window it does not hold.
            let coverage = if rows.first().is_some_and(|row| row.date <= from) {
                Coverage::complete(CoverageWindow {
                    asked_from: Some(from),
                    asked_to: Some(to),
                    returned: u64::try_from(rows.len()).unwrap_or(u64::MAX),
                    ..CoverageWindow::default()
                })
            } else {
                let read =
                    read_history_from_cache(&context.resources, tenant_id, user_id, from, to)
                        .await?;
                // Rows the stored sessions do warm answer the window, with the
                // read's coverage. When they warm none, what the rollup holds
                // still answers, and the coverage says so: the window is short,
                // and stands behind the rollup from its first row on — never
                // that nothing was computed, nor a first day past those rows.
                let mut coverage = match rows.first() {
                    Some(first) if read.states.is_empty() => Coverage {
                        completeness: Completeness::Incomplete(IncompleteReason::ShallowHistory),
                        trustworthy_from: Some(first.date),
                        ..read.coverage
                    },
                    _ => {
                        rows = read.states;
                        read.coverage
                    }
                };
                coverage.window.returned = u64::try_from(rows.len()).unwrap_or(u64::MAX);
                with_pending(&context.resources, (tenant_id, user_id), coverage).await?
            };

            // Every row carries its own form reading, and the payload carries
            // the method that produced it. This is the tool the endurance agent
            // prompt says to call first; shipping bare ctl/atl/tsb floats is how
            // a raw `-77` reached an athlete as a diagnosis nobody could explain
            // (registre#199).
            let params = &context.resources.cageux_config().algorithms.params;
            let days: Vec<TrainingHistoryDay> = rows
                .iter()
                .map(|row| {
                    let reading = row.form_reading();
                    TrainingHistoryDay {
                        state: row.clone(),
                        tsb_pct_of_ctl: reading.form_pct.map(f64::round),
                        form_band: reading.band,
                    }
                })
                .collect();
            let deep_fatigue_mornings = deep_fatigue_run(&days, to);
            let sources =
                delivering_services(&context.resources, tenant_id, user_id, from, to).await?;

            ok_typed(
                "get_training_history",
                TrainingHistoryResult {
                    from: from.format("%Y-%m-%d").to_string(),
                    to: to.format("%Y-%m-%d").to_string(),
                    days,
                    sources,
                    deep_fatigue_mornings,
                    coverage,
                    interpretation: FormReading::interpretation(
                        params.training_load_ctl_days,
                        params.training_load_atl_days,
                    ),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Build the Endurance training-history tool list for registry registration.
#[must_use]
pub fn create_endurance_history_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ComputeTrainingHistoryTool),
        Box::new(GetTrainingHistoryTool),
    ]
}

// Guardian security classifications (see `pierre_tool_runtime::security`). These
// tools read/compute the training history of the athlete the call reads — the
// caller, or a coached athlete for a coach's read — internal, no egress.
pierre_tool_runtime::declare_security!(ComputeTrainingHistoryTool => empty);
pierre_tool_runtime::declare_security!(GetTrainingHistoryTool => empty);

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: NaiveDate, band: FormBand, pct: f64) -> TrainingHistoryDay {
        TrainingHistoryDay {
            state: DailyTrainingState::zero(date),
            tsb_pct_of_ctl: Some(pct),
            form_band: band,
        }
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).expect("a valid day")
    }

    #[test]
    fn the_run_is_the_deep_fatigue_days_ending_on_to_oldest_first() {
        let days = [
            day(date(1), FormBand::DeepFatigue, -40.0),
            day(date(2), FormBand::HeavyBlock, -25.0),
            day(date(3), FormBand::DeepFatigue, -31.0),
            day(date(4), FormBand::DeepFatigue, -38.0),
        ];
        let run = deep_fatigue_run(&days, date(4));
        let dates: Vec<&str> = run.iter().map(|m| m.date.as_str()).collect();
        assert_eq!(
            dates,
            ["2026-11-03", "2026-11-04"],
            "the run stops at the first day out of deep fatigue"
        );
        assert_eq!(
            run[1].tsb_pct_of_ctl,
            Some(-38.0),
            "each day carries its share"
        );
    }

    #[test]
    fn no_run_when_to_has_no_row_or_a_day_is_missing() {
        let days = [
            day(date(2), FormBand::DeepFatigue, -40.0),
            day(date(4), FormBand::DeepFatigue, -38.0),
        ];
        assert!(
            deep_fatigue_run(&days, date(5)).is_empty(),
            "a series that stops short of `to` says nothing about today"
        );
        assert_eq!(
            deep_fatigue_run(&days, date(4)).len(),
            1,
            "a missing day ends the run: the 3rd has no row"
        );
    }
}
