// ABOUTME: HTTP integration tests for GET /api/personas — persona cards from the live contract registry
// ABOUTME: Covers FR rendering, strict-mode inheritance for coach, locale fallback, and the empty registry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! End-to-end tests for the persona cards endpoint. We stand up the
//! [`PersonasRoutes`] router with shared `ServerContext`, hydrate the
//! persona-contract registry with the SHIPPED contremaitre YAML (the
//! pinned rev resolved from `Cargo.lock`, exactly the document
//! production syncs), generate a JWT, and exercise the route through
//! `tower::ServiceExt::oneshot` so auth extraction and the error
//! envelope run exactly as in production.

mod common;
mod helpers;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::personas::PersonasRoutes;
use pierre_services::personas::resolve_persona_locale;

/// Read `config/persona_contracts.yaml` from the pinned dravr-contremaitre
/// checkout, resolved WITHOUT invoking cargo: a `cargo metadata` subprocess
/// inside a running test blocks on the package-cache lock whenever another
/// cargo process is alive — which in CI is always. Instead the locked rev
/// comes from the workspace `Cargo.lock` and the checkout directory is the
/// rev's 7-char prefix under `$CARGO_HOME/git/checkouts`, the same
/// resolution `scripts/ci/check-contremaitre-sync.sh` performs shell-side.
/// The checkout is guaranteed present wherever this test compiles, because
/// building the workspace required fetching it.
fn shipped_persona_contracts_yaml() -> String {
    let lock =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .expect("read workspace Cargo.lock");
    let rev = lock
        .lines()
        .find(|l| l.contains("dravr-ai/dravr-contremaitre") && l.contains('#'))
        .and_then(|l| l.rsplit('#').next())
        .map(|r| r.trim_end_matches('"').trim())
        .expect("dravr-contremaitre rev in Cargo.lock");
    let cargo_home = env::var("CARGO_HOME").map_or_else(
        |_| {
            PathBuf::from(env::var("HOME").expect("HOME set when CARGO_HOME is not")).join(".cargo")
        },
        PathBuf::from,
    );
    let checkouts = cargo_home.join("git/checkouts");
    let repo_dir = fs::read_dir(&checkouts)
        .unwrap_or_else(|e| panic!("read {}: {e}", checkouts.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dravr-contremaitre-"))
        })
        .expect("dravr-contremaitre checkout dir");
    let yaml_path = repo_dir
        .join(&rev[..7])
        .join("config/persona_contracts.yaml");
    fs::read_to_string(&yaml_path).unwrap_or_else(|e| panic!("read {}: {e}", yaml_path.display()))
}

/// Boot resources, hydrate the persona-contract registry with the shipped
/// contremaitre document, create an authenticated user (stored locale
/// defaults to "fr"), and mount the personas router.
async fn setup_with_shipped_contracts() -> (Router, String, Arc<ServerContext>) {
    let resources = create_test_server_resources().await.unwrap();
    resources
        .fitness
        .persona_contract_registry
        .apply_overlay(&shipped_persona_contracts_yaml())
        .expect("apply shipped persona contracts");
    let (_user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let router = PersonasRoutes::routes(Arc::clone(&resources));
    (router, format!("Bearer {token}"), resources)
}

#[tokio::test]
async fn fr_request_returns_four_personas_in_order_with_fr_rules() {
    let (router, auth, _resources) = setup_with_shipped_contracts().await;

    let response = AxumTestRequest::get("/api/personas?locale=fr")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    let personas = body["personas"].as_array().expect("personas array");
    assert_eq!(personas.len(), 4, "one card per CoachingPersona variant");

    let slugs: Vec<&str> = personas
        .iter()
        .map(|p| p["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        slugs,
        ["casual", "enthusiast", "power_athlete", "coach"],
        "cards must follow CoachingPersona enum order"
    );

    let names: Vec<&str> = personas
        .iter()
        .map(|p| p["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Casual", "Enthusiast", "Power-athlete", "Coach"],
        "display names are the canonical untranslated brand names"
    );

    // Casual: the shipped 150-word cap must render as an exact FR
    // sentence with the number interpolated from the live contract.
    let casual = &personas[0];
    assert_eq!(casual["summary"], "Amical, encourageant, sans jargon.");
    let casual_rules = casual["rules"].as_array().expect("casual rules");
    assert_eq!(casual_rules[0]["key"], "persona.rule.max_words");
    assert_eq!(casual_rules[0]["text"], "Réponses limitées à 150 mots.");
    let no_long_lists = casual_rules
        .iter()
        .find(|r| r["key"] == "persona.rule.no_long_lists")
        .expect("casual carries the shipped 3-item list rule");
    assert_eq!(
        no_long_lists["text"],
        "Pas de listes de 3 éléments ou plus."
    );

    // Enthusiast: its own cap (250) proves the number comes from each
    // persona's contract, not a shared constant.
    let enthusiast_rules = personas[1]["rules"].as_array().expect("enthusiast rules");
    assert_eq!(enthusiast_rules[0]["key"], "persona.rule.max_words");
    assert_eq!(enthusiast_rules[0]["text"], "Réponses limitées à 250 mots.");
}

#[tokio::test]
async fn enforcement_follows_strict_mode_including_coach_inheritance() {
    let (router, auth, _resources) = setup_with_shipped_contracts().await;

    let response = AxumTestRequest::get("/api/personas?locale=fr")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    let personas = body["personas"].as_array().expect("personas array");

    let enforcement_of = |slug: &str| {
        personas
            .iter()
            .find(|p| p["slug"] == slug)
            .unwrap_or_else(|| panic!("card for {slug}"))
    };

    assert_eq!(enforcement_of("casual")["enforcement"], "advisory");
    assert_eq!(enforcement_of("enthusiast")["enforcement"], "advisory");
    assert_eq!(enforcement_of("casual")["enforcement_label"], "Indicatif");

    assert_eq!(enforcement_of("power_athlete")["enforcement"], "verified");
    assert_eq!(
        enforcement_of("power_athlete")["enforcement_label"],
        "Vérifié à chaque réponse"
    );
    // Coach declares strict_mode: false in the YAML but inherits strict
    // from power_athlete through the registry's overlay (child || parent).
    // Reading the flattened snapshot — never re-deriving — is what this
    // asserts.
    assert_eq!(enforcement_of("coach")["enforcement"], "verified");

    // The inherited Power-athlete rules surface on the coach card too,
    // alongside coach-only roster framing.
    let coach_keys: Vec<&str> = enforcement_of("coach")["rules"]
        .as_array()
        .expect("coach rules")
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert!(coach_keys.contains(&"persona.rule.p0_p3_ladder"));
    assert!(coach_keys.contains(&"persona.rule.citations_required"));
    assert!(coach_keys.contains(&"persona.rule.athlete_attribution"));
    assert!(coach_keys.contains(&"persona.rule.roster_verified"));
    let roster_rule = enforcement_of("coach")["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "persona.rule.roster_verified")
        .unwrap();
    assert_eq!(
        roster_rule["text"],
        "Les citations d'athlètes sont vérifiées contre ton effectif."
    );
}

#[tokio::test]
async fn unknown_locale_falls_back_to_stored_locale_then_english() {
    let (router, auth, _resources) = setup_with_shipped_contracts().await;

    // An unsupported query locale falls back to the user's stored locale
    // (the test user keeps the "fr" profile default).
    let response = AxumTestRequest::get("/api/personas?locale=zz")
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(
        body["personas"][0]["summary"], "Amical, encourageant, sans jargon.",
        "unsupported ?locale must fall back to the stored profile locale"
    );

    // A supported query locale wins over the stored one.
    let response = AxumTestRequest::get("/api/personas?locale=en")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(
        body["personas"][0]["summary"], "Friendly, encouraging, no jargon.",
        "a supported ?locale must override the stored profile locale"
    );
    let rules = body["personas"][0]["rules"].as_array().unwrap();
    assert_eq!(rules[0]["text"], "Replies capped at 150 words.");

    // Terminal fallback: neither candidate supported resolves to English.
    assert_eq!(resolve_persona_locale(Some("zz"), Some("xx")), "en");
    assert_eq!(resolve_persona_locale(None, None), "en");
    assert_eq!(resolve_persona_locale(Some("de"), None), "de");
    assert_eq!(resolve_persona_locale(None, Some("pt")), "pt");
}

#[tokio::test]
async fn empty_registry_serves_cards_without_rules_never_500() {
    // No apply_overlay: this is the pre-first-sync boot state.
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let router = PersonasRoutes::routes(Arc::clone(&resources));

    let response = AxumTestRequest::get("/api/personas?locale=en")
        .header("authorization", &format!("Bearer {token}"))
        .send(router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "an empty registry must degrade, never 500"
    );
    let body: serde_json::Value = response.json();
    let personas = body["personas"].as_array().expect("personas array");
    assert_eq!(personas.len(), 4);
    for card in personas {
        assert_eq!(
            card["rules"].as_array().map(Vec::len),
            Some(0),
            "no contract yet means no rules on {}",
            card["slug"]
        );
        assert_eq!(card["enforcement"], "advisory");
        assert!(
            !card["summary"].as_str().unwrap().is_empty(),
            "summaries are compiled-in and survive an empty registry"
        );
    }
}

#[tokio::test]
async fn personas_without_auth_rejected() {
    let (router, _auth, _resources) = setup_with_shipped_contracts().await;

    let response = AxumTestRequest::get("/api/personas").send(router).await;

    assert!(
        response.status_code() == StatusCode::UNAUTHORIZED
            || response.status_code() == StatusCode::FORBIDDEN,
        "unauth request should be 401/403, got {}",
        response.status_code()
    );
}
