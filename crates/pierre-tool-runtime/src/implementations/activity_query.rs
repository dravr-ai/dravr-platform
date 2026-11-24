// ABOUTME: The query_activities tool: counts, sums and breakdowns of the athlete's workouts over any period
// ABOUTME: One read of the store through the aggregator, its segment half settled as get_segment_history settles it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `query_activities` (carnet#1485)
//!
//! The tool face of the one aggregator ([`crate::activity_query`]): it parses
//! the arguments ([`parse_query`]), counts through [`query_activities`] under
//! the model's gate, and, when a segment was asked for, settles the segment
//! half of the coverage exactly as `get_segment_history` does
//! ([`segment_completeness`]), the history half first.

use crate::read_subject::DataClass;
use crate::tool_data::ToolData;
use dravr_barrage::Stream;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::{CivilDate, Timestamp, Zone};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::civil_time::{clock_date, day_start_utc, resolve_zone};
use pierre_core::constants::tools::identifiers::QUERY_ACTIVITIES;
use pierre_core::errors::AppResult;
use pierre_core::models::coverage::{Completeness, IncompleteReason};
use pierre_core::models::TenantId;
use pierre_mcp_schema::PropertySchema;
use pierre_services::training_history_read::user_timezone;
use pierre_tools_core::ToolResult;
use serde_json::Value;

use crate::activity_query::{
    askable_tag_names, parse_query, query_activities, ActivityQuery, Athlete, QueryAnswer, ReadGate,
};
use crate::annotations::{annotations, Effect, Reach};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, format_property, object_schema, tool_definition, tool_result_to_response,
    Formatted,
};
use crate::implementations::data_helpers::parse_output_format;
use crate::implementations::handler_bridge;
use crate::implementations::segments::segment_completeness;
use crate::protocol::format::formatted_response;
use crate::runtime::ToolRuntime;

/// The athlete's civil `today`, on their own clock.
pub(super) async fn athlete_today(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
) -> AppResult<(CivilDate, Zone)> {
    let zone = resolve_zone(
        user_timezone(runtime.repos(), athlete.user_id)
            .await?
            .as_deref(),
    );
    Ok((clock_date(Timestamp::now(), &zone), zone))
}

/// Settle a segment count's coverage: the history half the aggregator
/// answered comes first (a history walk still copying the window in), then
/// the segment reason `get_segment_history` gives the same population.
///
/// The detail walk is asked over the window asked, or, with no `from`, from
/// the oldest workout counted: the history half already judged the open
/// floor on each source's own ladder.
async fn settle_segments(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    query: &ActivityQuery,
    zone: &Zone,
    answer: &mut QueryAnswer,
) {
    let Some(counts) = answer.segments else {
        return;
    };
    if answer.coverage.completeness
        == Completeness::Incomplete(IncompleteReason::HistoryWalkRunning)
    {
        return;
    }
    let floor_day = query
        .from
        .or(answer.coverage.window.oldest)
        .unwrap_or(query.to);
    let top_day = query.to.succ().unwrap_or(query.to);
    let sport = match (
        query.filter.sport_types.as_slice(),
        query.filter.sport_families.is_empty(),
    ) {
        ([one], true) => serde_json::to_value(one)
            .ok()
            .and_then(|name| name.as_str().map(str::to_owned)),
        _ => None,
    };
    let (completeness, pending) = segment_completeness(
        runtime,
        athlete,
        day_start_utc(floor_day, zone).epoch_seconds(),
        day_start_utc(top_day, zone).epoch_seconds(),
        sport.as_deref(),
        counts,
    )
    .await;
    if !completeness.is_complete() {
        answer.coverage.completeness = completeness;
        answer.coverage.pending = pending;
    }
}

fn property(property_type: &str, description: &str) -> PropertySchema {
    PropertySchema {
        property_type: property_type.to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

fn list_property(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "array".to_owned(),
        description: Some(description.to_owned()),
        items: Some(Box::new(property("string", "One value."))),
        ..Default::default()
    }
}

fn input_schema() -> pierre_mcp_schema::JsonSchema {
    let mut properties: HashMap<String, PropertySchema> = HashMap::new();
    properties.insert(
        "from".to_owned(),
        property("string", "YYYY-MM-DD; omit for all history."),
    );
    properties.insert(
        "to".to_owned(),
        property("string", "YYYY-MM-DD; default today."),
    );
    properties.insert(
        "sport_type".to_owned(),
        list_property("Sports, e.g. TrailRunning."),
    );
    properties.insert(
        "sport_family".to_owned(),
        list_property("running, cycling, swimming, other."),
    );
    properties.insert("place_id".to_owned(), list_property("Place ids."));
    properties.insert("name_contains".to_owned(), property("string", "Name text."));
    properties.insert(
        "min_distance_km".to_owned(),
        property("number", "Least km."),
    );
    properties.insert("max_distance_km".to_owned(), property("number", "Most km."));
    properties.insert(
        "has_segment".to_owned(),
        property("string", "Segment id/name."),
    );
    properties.insert("gear".to_owned(), property("string", "Gear id/name."));
    properties.insert(
        "workout_type".to_owned(),
        list_property(&format!("{}.", askable_tag_names().join(", "))),
    );
    properties.insert("trainer".to_owned(), property("boolean", "Indoor trainer."));
    properties.insert(
        "provider".to_owned(),
        property("string", "One source only."),
    );
    properties.insert(
        "group_by".to_owned(),
        list_property("Up to 2 of year, month, week, sport_type, sport_family, place, gear."),
    );
    properties.insert(
        "metrics".to_owned(),
        list_property("count, distance_km, moving_time_s, elapsed_time_s, elevation_m, avg_hr."),
    );
    properties.insert("format".to_owned(), format_property());
    object_schema(properties, None)
}

/// Tool counting, summing and breaking down the athlete's workouts over any
/// period.
#[derive(Debug)]
pub(super) struct QueryActivitiesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for QueryActivitiesTool {
    fn definition(&self) -> Tool {
        answers_with::<Formatted<QueryAnswer>>(tool_definition(
            "query_activities",
            "Count and sum workouts over any period, every source merged; for every count, \
             total or breakdown. Read coverage before quoting; unknown is never a no. If a \
             figure is disputed, call again and say which filter changed.",
            input_schema(),
            annotations("Query activities", Effect::ReadOnly, Reach::AthleteAccount),
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
            let athlete = Athlete {
                user_id: context.user_id,
                tenant_id: TenantId::from_uuid(context.require_tenant()?),
            };
            let runtime = &context.resources;
            let (today, zone) = athlete_today(runtime, athlete).await?;
            let query = parse_query(&args, today)?;
            let mut answer = query_activities(
                runtime,
                athlete.user_id,
                athlete.tenant_id,
                &query,
                ReadGate::Model,
            )
            .await?;
            settle_segments(runtime, athlete, &query, &zone, &mut answer).await;
            let response = formatted_response(
                &answer,
                parse_output_format(&args),
                HashMap::<_, Value>::new(),
            )?;
            handler_bridge::map_universal_response(QUERY_ACTIVITIES, Ok(response))
        }
        .await;
        tool_result_to_response(result)
    }
}

crate::declare_security!(
    QueryActivitiesTool => empty,
    reads: ToolData::of(DataClass::Training, &[Stream::ActivityDaily, Stream::Workout])
        .with_athlete_text(),
);
