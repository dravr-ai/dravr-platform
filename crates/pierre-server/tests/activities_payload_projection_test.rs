// ABOUTME: get_activities answers the same window three times over — only one copy reaches the model
// ABOUTME: The projection has to shrink the payload AND keep every field a chained activity_id call needs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user};
use embacle_tool_host::ToolSurface;
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::{ChatMessage, FunctionResponse};
use pierre_mcp_server::mcp::resources::tool_surface::TurnToolSurface;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::implementations::data_helpers::provider_reconnect_note;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::tool_execution::add_function_responses_to_messages;
use pierre_tool_runtime::tool_results::{format_tool_results_as_text, project_activities_payload};
use serde_json::{json, Value};
use uuid::Uuid;

/// A realistic `get_activities` envelope: the prose block the agent cites, the
/// structured array, the TOON copy, and the two sidecars — the shape
/// `build_activities_success_response` actually emits.
fn activities_envelope(rows: usize) -> Value {
    let activities: Vec<Value> = (0..rows)
        .map(|i| {
            json!({
                "id": format!("strava-{i}"),
                "name": format!("Morning Run {i}"),
                "sport_type": "run",
                "start_date": "2026-08-20T11:04:00Z",
                "start_date_local": "2026-08-20T07:04:00-04:00",
                "distance_meters": 12_345.6,
                "duration_seconds": 3_600,
                "elevation_gain_meters": 210.5,
                "average_heartrate": 148,
                "max_heartrate": 171,
                "average_speed": 3.42,
                "calories": 780,
                "description": "Felt strong through the back half, negative split.",
            })
        })
        .collect();

    json!({
        "activity_list": "2026-08-20 · Morning Run 0 · 12.3 km · 1h00 · 210 m\n",
        "activities": activities,
        "activities_toon": "id,name,sport_type\nstrava-0,Morning Run 0,run\n".repeat(rows),
        "provider": "strava",
        "count": rows,
        "mode": "summary",
        "format": "json",
        "offset": 0,
        "limit": 30,
        "has_more": true,
        "coverage": { "window_total": 552, "showing": rows },
        "token_estimate": { "tokens": 4_200, "chars": 16_800 },
        "retrieval_context": {
            "sufficiency": "adequate",
            "fragment_report": { "merged": 3, "note": "count sessions, not rows" },
        },
    })
}

#[test]
fn the_projection_keeps_every_field_a_chained_call_needs() {
    let projected = project_activities_payload("get_activities", &activities_envelope(3))
        .expect("a real get_activities envelope is recognised");

    // Five registered tools take a required `activity_id` and resolve it via
    // provider.get_activity(...) with no name-or-index fallback. Losing the id
    // would leave the agent able to describe a session and unable to analyse it.
    let rows = projected["activities"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        3,
        "every activity survives — this is not truncation"
    );
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["id"], json!(format!("strava-{i}")));
        assert_eq!(row["name"], json!(format!("Morning Run {i}")));
        assert_eq!(row["sport_type"], json!("run"));
        assert_eq!(row["start_date"], json!("2026-08-20T11:04:00Z"));
    }

    // The prose is what the agent cites, and the pagination scalars are what
    // the tool's own schema promises for a follow-up request.
    assert!(projected["activity_list"]
        .as_str()
        .unwrap()
        .contains("Morning Run 0"));
    assert_eq!(projected["provider"], json!("strava"));
    assert_eq!(projected["count"], json!(3));
    assert_eq!(projected["has_more"], json!(true));
    assert_eq!(projected["coverage"]["window_total"], json!(552));
}

#[test]
fn the_projection_drops_the_duplicate_copies_and_the_sidecars() {
    let projected = project_activities_payload("get_activities", &activities_envelope(3)).unwrap();

    assert!(
        projected.get("activities_toon").is_none(),
        "the TOON copy is a second rendering of the same window"
    );
    assert!(projected.get("token_estimate").is_none());
    assert!(projected.get("retrieval_context").is_none());

    // Per-activity detail bodies go; the addressing fields stay.
    let row = &projected["activities"][0];
    for dropped in [
        "distance_meters",
        "average_heartrate",
        "description",
        "calories",
    ] {
        assert!(
            row.get(dropped).is_none(),
            "{dropped} should not survive projection"
        );
    }
}

/// The one sidecar that survives, and the reason it has to.
///
/// A window served without a dead connection is a PARTIAL window. The tool
/// stamps `reconnect_required` into its own result to say so, and this
/// projection is the only thing between that stamp and the prompt: a key
/// absent from `ACTIVITIES_ENVELOPE_KEPT` reaches no model at all, so the agent
/// answers a short history as if it were the whole one.
#[test]
fn the_projection_carries_the_reconnect_sidecar_to_the_model() {
    let mut envelope = activities_envelope(3);
    envelope["reconnect_required"] = provider_reconnect_note("Garmin", "sciotte_garmin");

    let projected = project_activities_payload("get_activities", &envelope).unwrap();
    let caveat = &projected["reconnect_required"];
    assert_eq!(
        caveat["provider"],
        json!("Garmin"),
        "the model is told which source is missing, in the athlete's vocabulary"
    );
    assert!(
        caveat["note"]
            .as_str()
            .unwrap()
            .contains("served WITHOUT Garmin"),
        "the note that tells the coach not to imply a complete answer must survive"
    );

    // The text seam projects through the same allowlist, so it carries it too.
    let text = format_tool_results_as_text(&[FunctionResponse {
        name: "get_activities".to_owned(),
        response: envelope,
    }]);
    assert!(
        text.contains("reconnect_required") && text.contains("served WITHOUT Garmin"),
        "the text tool loop's rendering must name the dead source: {text}"
    );
}

#[test]
fn a_thirty_activity_window_shrinks_by_more_than_half() {
    let full = activities_envelope(30);
    let projected = project_activities_payload("get_activities", &full).unwrap();

    let before = serde_json::to_string(&full).unwrap().len();
    let after = serde_json::to_string(&projected).unwrap().len();

    assert!(
        after * 2 < before,
        "projection should more than halve a 30-activity window; {before} -> {after}"
    );
}

#[test]
fn any_other_tool_passes_through_untouched() {
    let payload = json!({ "total_distance_km": 1234.5, "activity_list": "not an activities call" });
    assert!(
        project_activities_payload("get_stats", &payload).is_none(),
        "only get_activities is projected"
    );
}

#[test]
fn an_unrecognised_shape_passes_through_untouched() {
    // No `activity_list` — an error envelope, or a future rewrite. The reducer
    // must never be the reason an agent ends up with no data.
    let error_shape = json!({ "error": "provider unavailable", "activities": [] });
    assert!(project_activities_payload("get_activities", &error_shape).is_none());

    // `activity_list` present but not a string is equally unrecognised.
    let wrong_type = json!({ "activity_list": ["a", "b"], "activities": [] });
    assert!(project_activities_payload("get_activities", &wrong_type).is_none());
}

#[test]
fn the_api_loop_injects_the_projected_payload_not_the_whole_envelope() {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let responses = vec![FunctionResponse {
        name: "get_activities".to_owned(),
        response: activities_envelope(30),
    }];

    let added = add_function_responses_to_messages(&mut messages, &responses);

    let injected = &messages.last().unwrap().content;
    assert!(injected.contains("strava-0"), "ids reach the model");
    assert!(injected.contains("Morning Run 0"));
    assert!(
        !injected.contains("retrieval_context"),
        "the sidecar must not reach the prompt"
    );
    assert!(
        !injected.contains("activities_toon"),
        "the duplicate rendering must not reach the prompt"
    );
    assert!(
        !injected.contains("Felt strong through the back half"),
        "per-activity detail bodies must not reach the prompt"
    );

    // The prepended list is unaffected — it is read off the ORIGINAL response,
    // so projecting the injected copy cannot cost the reply its activity list.
    assert!(added.activity_list.is_some());
}

#[test]
fn the_text_loop_and_the_recovery_reask_share_the_same_projection() {
    let responses = vec![FunctionResponse {
        name: "get_activities".to_owned(),
        response: activities_envelope(30),
    }];

    // embacle pretty-prints these blocks, so this seam was strictly more
    // expensive than the API loop's compact serialization for identical data.
    let text = format_tool_results_as_text(&responses);
    assert!(text.contains("<tool_result name=\"get_activities\">"));
    assert!(
        text.contains("strava-0"),
        "ids survive the text rendering too"
    );
    assert!(!text.contains("retrieval_context"));
    assert!(!text.contains("activities_toon"));
    assert!(!text.contains("Felt strong through the back half"));
}

// ============================================================================
// The fourth seam — the loopback MCP surface (registre#128)
// ============================================================================

/// An athlete whose `get_activities` answers a real window with no network.
///
/// Two rides sit in the durable cache under a Strava connection, and a Garmin
/// connection with no session is elected primary: the tool cannot authenticate
/// it and serves the window from the sibling's cache instead.
async fn athlete_with_cached_rides(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let rides: Vec<_> = [
        ("strava-ride-1", "Sortie longue", 2),
        ("strava-ride-2", "Tempo", 4),
    ]
    .into_iter()
    .map(|(id, name, days_ago)| {
        ActivityBuilder::new(
            id.to_owned(),
            name.to_owned(),
            SportType::Ride,
            Utc::now() - Duration::days(days_ago),
            7_200,
            "strava".to_owned(),
        )
        .distance_meters(80_000.0)
        .build()
    })
    .collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &rides)
        .await
        .unwrap();

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    (user_id, tenant)
}

/// The executor a loopback turn dispatches through, as `turn_surface` builds it.
fn loopback_executor(resources: &Arc<ServerContext>) -> Arc<UniversalToolExecutor> {
    Arc::new(
        UniversalToolExecutor::new(resources.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_conversation_id("conv-projection".into()),
    )
}

/// The surface an ACP agent calls Dravr's tools through, for one turn.
fn loopback_surface(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> TurnToolSurface {
    TurnToolSurface::new(
        resources.mcp.tool_registry.clone(),
        resources.common.repos.clone(),
        loopback_executor(resources),
        user_id.to_string(),
        tenant,
        64,
    )
}

/// What the same tool returns when nothing stands between it and the caller:
/// the envelope the loopback seam receives before it decides what to forward.
async fn whole_payload(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    tool_name: &str,
    parameters: Value,
) -> Value {
    let response = loopback_executor(resources)
        .execute_tool(UniversalRequest {
            tool_name: tool_name.to_owned(),
            parameters,
            user_id: user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(response.success, "{tool_name} ran: {:?}", response.error);
    response.result.expect("the tool returned a payload")
}

/// The seam production actually runs must project, like the other three.
///
/// `copilot_headless` never reports `FUNCTION_CALLING`, so `tool_dispatch` takes
/// the loopback branch — which makes this the live path and the other three the
/// fallbacks. The agent re-sends what this seam hands it on every pass of its
/// own loop, so a whole envelope here is the whole window, paid for repeatedly.
#[tokio::test]
async fn the_loopback_seam_projects_the_activities_payload() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_cached_rides(&resources).await;
    let arguments = json!({ "limit": 10, "mode": "summary" });

    // The fixture has to carry what the projection removes, or the assertions
    // below pass on a payload that never had anything to drop.
    let whole = whole_payload(
        &resources,
        user_id,
        tenant,
        "get_activities",
        arguments.clone(),
    )
    .await;
    assert!(
        whole["activities"][0].get("distance_meters").is_some(),
        "the unprojected envelope carries per-activity detail: {whole}"
    );
    assert!(
        whole.get("token_estimate").is_some() || whole.get("retrieval_context").is_some(),
        "the unprojected envelope carries a sidecar: {whole}"
    );

    let outcome = loopback_surface(&resources, user_id, tenant)
        .call("get_activities", &arguments)
        .await;
    assert!(!outcome.is_error, "the call succeeded: {}", outcome.text);
    let delivered = outcome
        .structured
        .expect("a served window reaches the agent as structured content");

    let rows = delivered["activities"]
        .as_array()
        .expect("activities array");
    let ids: Vec<&str> = rows.iter().filter_map(|row| row["id"].as_str()).collect();
    assert_eq!(
        ids,
        vec!["strava-ride-1", "strava-ride-2"],
        "every activity keeps the id a chained activity_id call needs"
    );
    for row in rows {
        let fields: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            fields
                .iter()
                .all(|field| ["id", "name", "sport_type", "start_date"].contains(field)),
            "only the addressing fields cross the loopback seam, got {fields:?}"
        );
    }
    assert!(
        delivered["activity_list"].is_string(),
        "the prose the agent cites survives"
    );
    for dropped in ["activities_toon", "token_estimate", "retrieval_context"] {
        assert!(
            delivered.get(dropped).is_none(),
            "{dropped} must not cross the loopback seam"
        );
    }
    // The text is what a model without structured-content support reads.
    assert!(
        !outcome.text.contains("distance_meters"),
        "per-activity detail must not reach the agent in the text either: {}",
        outcome.text
    );
}

/// An unrecognised shape must still travel whole.
///
/// The projection declining has to fall back to the original payload, not to
/// null or an empty object. This is the half that keeps the reducer from ever
/// being the reason an agent has no data.
#[tokio::test]
async fn the_loopback_seam_passes_unrecognised_payloads_through() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_cached_rides(&resources).await;

    let whole = whole_payload(
        &resources,
        user_id,
        tenant,
        "list_fitness_configs",
        json!({}),
    )
    .await;
    assert!(
        whole.as_object().is_some_and(|payload| !payload.is_empty()),
        "the fixture tool answers with a real payload: {whole}"
    );

    let outcome = loopback_surface(&resources, user_id, tenant)
        .call("list_fitness_configs", &json!({}))
        .await;
    assert!(!outcome.is_error, "the call succeeded: {}", outcome.text);
    // Two dispatches stamp two instants; everything else must be identical.
    let without_stamp = |mut payload: Value| {
        payload
            .as_object_mut()
            .expect("an object payload")
            .remove("retrieved_at");
        payload
    };
    assert_eq!(
        outcome.structured.map(without_stamp),
        Some(without_stamp(whole)),
        "a payload the projection does not recognise crosses the seam untouched"
    );

    // And the projection does decline for anything that is not the envelope.
    assert!(
        project_activities_payload("get_activities", &json!({"error": "no provider"})).is_none(),
        "an error envelope carries no activity_list and must not be projected"
    );
    assert!(
        project_activities_payload("get_athlete", &activities_envelope(3)).is_none(),
        "the projection is scoped to get_activities by name"
    );
}
