// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The store reads a coach in the athlete's language — the coach_translations overlay reaches browse, detail and search
// ABOUTME: An English reader keeps the canonical row; a French reader gets the French title and description contremaitre ships

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::Value;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::coach_fixtures::publish_catalogue_coach;
use pierre_database::seed_models::SeedCoachTranslation;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_coaches::build_store_router;

async fn get_json(resources: &Arc<ServerContext>, token: &str, path: &str) -> Value {
    let router = build_store_router::<ServerContext>().with_state(Arc::clone(resources));
    let resp = AxumTestRequest::get(path)
        .header("Authorization", token)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{path}");
    resp.json()
}

fn title_of<'a>(coaches: &'a [Value], id: &str) -> &'a str {
    coaches
        .iter()
        .find(|c| c["id"] == id)
        .and_then(|c| c["title"].as_str())
        .unwrap_or_else(|| panic!("coach {id} missing from {coaches:?}"))
}

#[tokio::test]
async fn the_store_reads_a_coach_in_the_athletes_language() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;

    let coach_id = publish_catalogue_coach(
        &repos,
        user_id,
        tenant_id,
        "Marathon Coach",
        "You coach marathons.",
    )
    .await;
    repos
        .seeder
        .seed_upsert_coach_translation(&SeedCoachTranslation {
            coach_id: coach_id.to_string(),
            locale: "fr".to_owned(),
            title: Some("Coach marathon".to_owned()),
            description: Some("Pour courir loin, longtemps.".to_owned()),
            purpose: None,
            instructions: None,
            source_sha: None,
            // The locale file declares its own chips; a coach without them
            // keeps the English tags, which the second case below pins.
            tags: Some(vec!["marathon".to_owned(), "endurance".to_owned()]),
        })
        .await
        .expect("translation row");
    let id = coach_id.to_string();

    // An English reader keeps the canonical row.
    repos
        .users
        .update_locale(user_id, "en")
        .await
        .expect("set en");
    let browse = get_json(&resources, &token, "/api/store/coaches").await;
    assert_eq!(
        title_of(browse["coaches"].as_array().unwrap(), &id),
        "Marathon Coach"
    );

    // A French reader gets the overlay on browse, detail and search alike.
    repos
        .users
        .update_locale(user_id, "fr")
        .await
        .expect("set fr");
    let browse = get_json(&resources, &token, "/api/store/coaches").await;
    assert_eq!(
        title_of(browse["coaches"].as_array().unwrap(), &id),
        "Coach marathon"
    );

    let detail = get_json(&resources, &token, &format!("/api/store/coaches/{id}")).await;
    assert_eq!(detail["title"], "Coach marathon");
    assert_eq!(detail["description"], "Pour courir loin, longtemps.");
    // The tag chips are the locale's own words, not the English fixture's.
    assert_eq!(
        detail["tags"]
            .as_array()
            .expect("detail carries tags")
            .iter()
            .map(|tag| tag.as_str().expect("a tag is a string"))
            .collect::<Vec<_>>(),
        vec!["marathon", "endurance"]
    );

    let search = get_json(&resources, &token, "/api/store/search?q=marathon").await;
    assert_eq!(
        title_of(search["coaches"].as_array().unwrap(), &id),
        "Coach marathon"
    );
}

/// A translation that declares no tags leaves the English chips visible,
/// rather than blanking them — partial translations are the norm.
#[tokio::test]
async fn a_translation_without_tags_keeps_the_english_chips() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;

    let coach_id = publish_catalogue_coach(
        &repos,
        user_id,
        tenant_id,
        "Recovery Coach",
        "You coach recovery.",
    )
    .await;
    repos
        .seeder
        .seed_upsert_coach_translation(&SeedCoachTranslation {
            coach_id: coach_id.to_string(),
            locale: "fr".to_owned(),
            title: Some("Coach récupération".to_owned()),
            description: None,
            purpose: None,
            instructions: None,
            source_sha: None,
            tags: None,
        })
        .await
        .expect("translation row");

    repos
        .users
        .update_locale(user_id, "fr")
        .await
        .expect("set fr");
    let detail = get_json(
        &resources,
        &token,
        &format!("/api/store/coaches/{coach_id}"),
    )
    .await;
    assert_eq!(detail["title"], "Coach récupération");
    assert_eq!(
        detail["tags"]
            .as_array()
            .expect("detail carries tags")
            .iter()
            .map(|tag| tag.as_str().expect("a tag is a string"))
            .collect::<Vec<_>>(),
        vec!["test"],
        "an untranslated tag list stays as the canonical English one"
    );
}
