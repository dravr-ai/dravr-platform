// ABOUTME: The get_activities tool's MCP definition: its arguments and their descriptions.
// ABOUTME: Kept apart from the execution path so the schema reads on its own.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use dravr_tronc::mcp::schema::Tool;
use pierre_mcp_schema::PropertySchema;

use crate::annotations::{annotations, Effect, Reach};
use crate::conversions::{
    answers_with, format_property, object_schema, task_capable, tool_definition,
};
use crate::implementations::activities_output::GetActivitiesResult;
use crate::protocol::provider_helpers::PROVIDER_ARGUMENT;

/// The definition `GetActivitiesTool` advertises: every argument is optional.
pub(crate) fn get_activities_definition() -> Tool {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(PROVIDER_ARGUMENT.to_owned()),
            ..Default::default()
        },
    );

    properties.insert(
        "limit".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some(
                "Maximum activities (1-400): the smallest that answers the question, e.g. 1 for 'last activity', 5-10 for 'this week', 20 for broader queries. Do NOT raise it to reach older activities (the feed is newest-first); use `after`/`before`. has_more flags a next page.".to_owned(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "offset".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some("Activities to skip, for paging.".to_owned()),
            ..Default::default()
        },
    );

    properties.insert(
        "sport_type".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Sport type, e.g. 'run', 'ride', 'swim'; case-insensitive.".to_owned(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "before".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some(
                "Upper bound, epoch seconds. Pair with `after` for a year or date range (all of 2022 = after 1640995200, before 1672531200).".to_owned(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "after".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some(
                "Lower bound, epoch seconds. REQUIRED for any year- or date-scoped question ('my 2022 races', 'runs last spring'): set it to the range start and `before` to its end; date bounds are the only way to query history. A deep historical window may first return a 'fetching, ask again shortly' status.".to_owned(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "mode".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "'summary' (default, minimal fields) or 'detailed' (full activity data)."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "analysis_type".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Tunes the data-sufficiency guidance: 'weekly_summary', 'trend_analysis', 'race_preparation', 'recovery_assessment' or 'general_overview' (default).".to_owned(),
            ),
            ..Default::default()
        },
    );

    properties.insert("format".to_owned(), format_property());

    properties.insert(
        "sort_by".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Applied BEFORE `limit`, so the truncated set keeps the right activities: 'date_desc' (default, newest first), 'date_asc' (oldest first), 'distance_desc' (longest first), 'distance_asc', 'duration_desc' (longest time first) or 'duration_asc'. 'longest to shortest' / 'de la plus longue à la plus courte' => distance_desc; 'oldest first' / 'du début' => date_asc.".to_owned(),
            ),
            ..Default::default()
        },
    );

    // All parameters are optional.
    let schema = object_schema(properties, None);

    answers_with::<GetActivitiesResult>(task_capable(tool_definition(
        "get_activities",
        "Retrieve the user's activities from connected providers. For a year or date range \
         (e.g. '2022 races'), pass `after`/`before`; do NOT page recent activities with \
         `limit` to reach old data. Use `sort_by` for an explicit ordering request (e.g. \
         longest to shortest).",
        schema,
        annotations("Get activities", Effect::ReadOnly, Reach::AthleteAccount),
    )))
}
