// ABOUTME: MCP tool schema definitions for fitness data and provider connection tools
// ABOUTME: Covers get_activities, get_athlete, get_stats, activity intelligence, and provider connections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::constants::{
    json_fields::{ACTIVITY_ID, AFTER, BEFORE, FORMAT, LIMIT, MODE, OFFSET, PROVIDER, SPORT_TYPE},
    tools::{
        CONNECT_PROVIDER, DISCONNECT_PROVIDER, GET_ACTIVITIES, GET_ACTIVITY_INTELLIGENCE,
        GET_ATHLETE, GET_CONNECTION_STATUS, GET_STATS,
    },
};

use super::{format_property, JsonSchema, PropertySchema, ToolSchema};

/// Create fitness data and provider connection tool schemas
pub(super) fn create_fitness_tools() -> Vec<ToolSchema> {
    vec![
        // Connection tools
        // Note: connect_to_pierre removed - SDK bridge handles it locally via RFC 8414 discovery
        create_connect_provider_tool(),
        create_get_connection_status_tool(),
        create_disconnect_provider_tool(),
        // Original tools
        create_get_activities_tool(),
        create_get_athlete_tool(),
        create_get_stats_tool(),
        create_get_activity_intelligence_tool(),
    ]
}

/// Create the `get_activities` tool schema
fn create_get_activities_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        PROVIDER.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name (e.g., 'strava', 'fitbit')".into()),
        },
    );

    properties.insert(
        LIMIT.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of activities to return. Safe limits to avoid context overflow: format=toon + mode=summary: ≤300, format=toon + mode=detailed: ≤30, format=json + mode=summary: ≤150, format=json + mode=detailed: ≤15".into()),
        },
    );

    properties.insert(
        OFFSET.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of activities to skip (for pagination)".into()),
        },
    );

    properties.insert(
        BEFORE.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "Unix timestamp (seconds) - return activities before this time".into(),
            ),
        },
    );

    properties.insert(
        AFTER.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "Unix timestamp (seconds) - return activities after this time. If not specified, defaults to 90 days ago to prevent context overflow.".into(),
            ),
        },
    );

    properties.insert(
        MODE.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Response detail level: 'summary' returns compact data (id, name, sport_type, start_date, distance_meters, duration_seconds) - use for listing/browsing many activities. 'detailed' returns full activity data with GPS, segments, laps - use only when analyzing a specific activity. Default: 'summary'. WARNING: 'detailed' mode with many activities will overflow LLM context.".into(),
            ),
        },
    );

    properties.insert(
        SPORT_TYPE.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Filter by sport type (e.g., 'NordicSki', 'Run', 'Ride', 'Swim'). Case-insensitive. Returns only activities matching this sport type.".into(),
            ),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: GET_ACTIVITIES.to_owned(),
        description: "Get fitness activities from a provider. Use mode='summary' (default) for listing activities - returns compact data safe for LLM context. Use mode='detailed' only for single activity analysis. Combine with before/after timestamps and sport_type filter to efficiently query large date ranges. Response metadata includes pagination info (offset, limit, returned_count, has_more) to enable intelligent pagination through large result sets. Response includes token_estimate with estimated_tokens, context_usage_percent, and guidance for managing LLM context limits. Default: 90-day time window applied when 'after' not specified.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![PROVIDER.to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `get_athlete` tool schema
fn create_get_athlete_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        PROVIDER.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name (e.g., 'strava', 'fitbit')".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: GET_ATHLETE.to_owned(),
        description: "Get athlete profile from a provider".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![PROVIDER.to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `get_stats` tool schema
fn create_get_stats_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        PROVIDER.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name (e.g., 'strava', 'fitbit')".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: GET_STATS.to_owned(),
        description: "Get fitness statistics from a provider".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![PROVIDER.to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `get_activity_intelligence` tool schema
fn create_get_activity_intelligence_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        PROVIDER.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name (e.g., 'strava', 'fitbit')".into()),
        },
    );

    properties.insert(
        ACTIVITY_ID.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the specific activity to analyze".into()),
        },
    );

    properties.insert(
        "include_weather".into(),
        PropertySchema {
            property_type: "boolean".into(),
            description: Some("Whether to include weather analysis (default: true)".into()),
        },
    );

    properties.insert(
        "include_location".into(),
        PropertySchema {
            property_type: "boolean".into(),
            description: Some("Whether to include location intelligence (default: true)".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: GET_ACTIVITY_INTELLIGENCE.to_owned(),
        description: "Generate AI-powered insights and analysis for a specific activity".to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![PROVIDER.to_owned(), ACTIVITY_ID.to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `connect_provider` tool schema
fn create_connect_provider_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    // Provider parameter (required)
    properties.insert(
        "provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Fitness provider to connect to. Supported providers: 'strava', 'fitbit'".into(),
            ),
        },
    );

    // Redirect URL parameter (optional, for mobile apps)
    properties.insert(
        "redirect_url".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Optional redirect URL for mobile OAuth flows. After OAuth completes, the server redirects to this URL with success/error params. Allowed schemes: pierre://, exp://, http://localhost, https://".into(),
            ),
        },
    );

    ToolSchema {
        name: CONNECT_PROVIDER.to_owned(),
        description: "Connect to Fitness Provider - Unified authentication flow that connects you to both Pierre and a fitness provider (like Strava or Fitbit) in a single seamless process. This will open a browser window for secure authentication with both systems.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `get_connection_status` tool schema
fn create_get_connection_status_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    // Optional OAuth credentials for Strava
    properties.insert(
        "strava_client_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Optional: Your Strava OAuth client ID. If provided with client_secret, will be used instead of server defaults.".into()),
        },
    );

    properties.insert(
        "strava_client_secret".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Optional: Your Strava OAuth client secret. Must be provided with client_id."
                    .into(),
            ),
        },
    );

    // Optional OAuth credentials for Fitbit
    properties.insert(
        "fitbit_client_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Optional: Your Fitbit OAuth client ID. If provided with client_secret, will be used instead of server defaults.".into()),
        },
    );

    properties.insert(
        "fitbit_client_secret".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Optional: Your Fitbit OAuth client secret. Must be provided with client_id."
                    .into(),
            ),
        },
    );

    ToolSchema {
        name: GET_CONNECTION_STATUS.to_owned(),
        description: "Check which fitness providers are currently connected and authorized for the user. Returns connection status for all supported providers. Optionally accepts OAuth credentials to use custom apps instead of server defaults.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `disconnect_provider` tool schema
fn create_disconnect_provider_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        PROVIDER.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider to disconnect (e.g., 'strava', 'fitbit')".into()),
        },
    );

    ToolSchema {
        name: DISCONNECT_PROVIDER.to_owned(),
        description: "Disconnect and remove stored tokens for a specific fitness provider. This revokes access to the provider's data.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![PROVIDER.to_owned()]),
        },
        annotations: None,
    }
}
