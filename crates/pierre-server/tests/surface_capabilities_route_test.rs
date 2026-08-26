// ABOUTME: Integration test for GET /api/surfaces/capabilities — the generated catalogue's source
// ABOUTME: Asserts concrete rows, so a returns-empty catalogue fails here instead of shipping

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The catalogue endpoint.
//!
//! Everything a client generates from comes through this route, so the
//! assertions are on real values — a surface count, a transport ceiling, the
//! block list of a channel that cannot draw — rather than on the request
//! succeeding. A catalogue that answered `{"surfaces": []}` would satisfy a
//! status-code test and generate an empty constant file.

#![cfg(all(feature = "client-chat", feature = "client-messaging"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::body::{to_bytes, Body};
use axum::http::{Request as HttpRequest, StatusCode};
use pierre_core::models::notifications::NotificationScreen;
use pierre_mcp_server::routes::surfaces::{SurfaceCapabilitiesResponse, SurfaceRoutes};
use serde_json::Value;
use std::error::Error;
use tower::ServiceExt;

/// Response-body size ceiling for `to_bytes`; the catalogue is a few KB.
const BODY_LIMIT: usize = 256 * 1024;

/// Fetch the catalogue's raw bytes through the real router.
async fn catalogue_bytes() -> Result<Vec<u8>, Box<dyn Error>> {
    let response = SurfaceRoutes::routes()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/surfaces/capabilities")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(to_bytes(response.into_body(), BODY_LIMIT).await?.to_vec())
}

/// Fetch the catalogue as free-form JSON.
async fn catalogue() -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&catalogue_bytes().await?)?)
}

/// Pull one surface row out of the catalogue.
fn surface<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["surfaces"]
        .as_array()
        .expect("surfaces is an array")
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("no {id} row in the catalogue"))
}

/// Read a row's block list as strings.
fn blocks(row: &Value) -> Vec<String> {
    row["blocks"]
        .as_array()
        .expect("blocks is an array")
        .iter()
        .map(|kind| kind.as_str().expect("block kind is a string").to_owned())
        .collect()
}

#[tokio::test]
async fn catalogue_covers_every_surface() -> Result<(), Box<dyn Error>> {
    let body = catalogue().await?;
    let ids: Vec<&str> = body["surfaces"]
        .as_array()
        .expect("surfaces is an array")
        .iter()
        .map(|row| row["id"].as_str().expect("id is a string"))
        .collect();
    assert_eq!(
        ids,
        vec![
            "web_chat",
            "mobile_chat",
            "telegram",
            "whatsapp",
            "discord",
            "slack",
            "messenger"
        ]
    );
    Ok(())
}

#[tokio::test]
async fn the_in_app_rows_are_identical_and_complete() -> Result<(), Box<dyn Error>> {
    let body = catalogue().await?;
    let web = surface(&body, "web_chat");
    let mobile = surface(&body, "mobile_chat");

    assert_eq!(blocks(web), blocks(mobile));
    assert_eq!(
        blocks(web),
        vec![
            "prose",
            "activity_list",
            "workout_plan",
            "scene",
            "verdicts",
            "reconnect",
            "actions",
            "notice"
        ]
    );
    assert_eq!(web["prose"], "markdown");
    // No transport ceiling: an absence on the wire, not a number.
    assert!(web["max_reply_chars"].is_null());
    assert_eq!(web["progressive"], "delta_channel");
    assert_eq!(web["streams_text_deltas"], true);
    assert_eq!(web["model_policy"], "use_stored");
    assert!(web["max_tool_iterations"].is_null());
    assert_eq!(web["call_type"], "chat");
    Ok(())
}

#[tokio::test]
async fn messaging_rows_carry_their_real_transport_ceilings() -> Result<(), Box<dyn Error>> {
    let body = catalogue().await?;

    let telegram = surface(&body, "telegram");
    assert_eq!(telegram["max_reply_chars"], 4096);
    assert_eq!(telegram["prose"], "plain_text");
    assert_eq!(telegram["progressive"], "complete");
    assert_eq!(telegram["streams_text_deltas"], false);
    assert_eq!(telegram["max_tool_iterations"], 5);
    assert_eq!(telegram["model_policy"], "override_with_env");
    assert_eq!(telegram["call_type"], "messaging");
    assert!(
        !blocks(telegram).contains(&"scene".to_owned()),
        "a chat message has no drawing surface"
    );
    assert!(blocks(telegram).contains(&"scene_image".to_owned()));

    let discord = surface(&body, "discord");
    assert_eq!(discord["max_reply_chars"], 2000);

    let slack = surface(&body, "slack");
    assert_eq!(slack["max_reply_chars"], 40_000);
    Ok(())
}

#[tokio::test]
async fn block_kinds_and_screens_are_published_whole() -> Result<(), Box<dyn Error>> {
    let body = catalogue().await?;

    let kinds: Vec<&str> = body["block_kinds"]
        .as_array()
        .expect("block_kinds is an array")
        .iter()
        .map(|kind| kind.as_str().expect("kind is a string"))
        .collect();
    assert_eq!(kinds.len(), 9);
    assert!(kinds.contains(&"prose"));
    assert!(kinds.contains(&"scene_image"));

    let screens = body["notification_screens"]
        .as_array()
        .expect("notification_screens is an array");
    assert_eq!(screens.len(), NotificationScreen::all().len());
    let connections = screens
        .iter()
        .find(|row| row["screen"] == "connections")
        .expect("the provider-reauth screen is catalogued");
    // The screen neither client's hand-written map handled.
    assert_eq!(connections["surface"], "data-providers");
    let coach = screens
        .iter()
        .find(|row| row["screen"] == "coach")
        .expect("the coach screen is catalogued");
    assert_eq!(coach["surface"], "chat");
    Ok(())
}

#[tokio::test]
async fn the_body_matches_the_response_type_field_for_field() -> Result<(), Box<dyn Error>> {
    // The generator reads this document by field name. Parsing it back into the
    // response type is what proves the names on the wire are the names the
    // server declares — a serde rename or a dropped field would still produce
    // valid JSON that the free-form assertions above could be written around.
    let typed: SurfaceCapabilitiesResponse = serde_json::from_slice(&catalogue_bytes().await?)?;
    assert_eq!(typed.surfaces.len(), 7);
    assert_eq!(typed.block_kinds.len(), 9);
    assert_eq!(
        typed.notification_screens.len(),
        NotificationScreen::all().len()
    );

    let web = typed
        .surfaces
        .iter()
        .find(|row| row.id == "web_chat")
        .expect("the web row survives the round trip");
    assert_eq!(web.blocks.len(), 8);
    assert_eq!(web.max_reply_chars, None);
    assert!(web.interactive);
    Ok(())
}
