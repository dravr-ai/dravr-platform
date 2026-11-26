// ABOUTME: The step-up approval of a one-off T2 question through the real authorize and consent routes
// ABOUTME: A fresh sign-in on Dravr's page, a live parent, a T2 host: a child grant the parent's code names

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Step-up (owl phase 5, slice 4b)
//!
//! A `one_off_question` request is refused before sign-in while the
//! catalogue does not word the step-up screen. Lit (the screen's lines laid
//! into the registry, as the picker suites do), it is approved only after a
//! sign-in on the authorization server's own page at most five minutes old,
//! under a live parent grant, for a client declaring a Host Profile T2; the
//! approval records a child the parent's code names. The child ends with its
//! parent, alone when revoked, and once when consumed.

use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::hidden_field;
use crate::oauth2_grant_details_test::{
    access_detail, app, approve_with, challenge, code_of, constraints_detail, fresh_state,
    live_grants, register, request_query, world, Client, World, REDIRECT,
};

use dravr_tronc::dates::Timestamp;
use pierre_core::models::{ChildGrant, GrantActor, GrantEvent, OWNER_VERIFIED_ACR};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

/// The step-up screen's three lines, as the catalogue will word them.
const STEP_UP_LINES: [(&str, &str); 3] = [
    (
        "oauth.consent.stepUpTitle",
        "{0} asks for a one-off look at your health records",
    ),
    (
        "oauth.consent.stepUpT2Disclosure",
        "This includes your health records. {0} reads them once, then this access ends.",
    ),
    (
        "oauth.consent.stepUpHost",
        "{0} says it handles health records with care. Dravr cannot check this.",
    ),
];

/// Word the step-up screen in English, the athlete's language.
fn light_step_up(world: &World) {
    let strings = &world.resources.mcp.messaging_strings_registry;
    for (key, line) in STEP_UP_LINES {
        let sha = format!("{:x}", Sha256::digest(line.as_bytes()));
        strings.update(key, "en", line.to_owned(), sha.clone());
        strings.update(key, &world.user.locale, line.to_owned(), sha);
    }
}

/// The client declares a Host Profile T2, as its CIMD document would.
async fn declare_t2_host(world: &World, client: &Client) {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .set_client_host_profile(&client.id, Some(r#"{"tier_max":"T2","version":1}"#))
        .await
        .unwrap();
}

/// A one-off question about the athlete's labs.
fn one_off_details(world: &World) -> String {
    json!([
        access_detail(
            world,
            &json!({
                "selection_preset": "one_off_question",
                "access_mode": "single_use",
                "streams": [{"name": "health.vault.labs.summary", "necessity": "required"}],
            })
        ),
        constraints_detail(&json!({
            "max_tier": "T2",
            "step_up": {"required": true, "acr": "owner_verified"},
            "host_profile_required": "t2",
        })),
    ])
    .to_string()
}

/// The authorization server's own sign-in cookie, signed in now.
fn hosted_sign_in(world: &World) -> String {
    let session = world
        .resources
        .auth
        .auth_manager
        .generate_authorization_session_token(
            &world.user,
            &world.resources.auth.jwks_manager,
            Some(world.tenant_id.clone()),
        )
        .unwrap();
    format!("pierre_session={session}")
}

/// The athlete approves `client`'s parent grant on the picker: training coach.
async fn approve_parent(world: &World, client: &Client) -> String {
    let approved = approve_with(world, client, &[], Some("training_coach")).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    code_of(&approved);
    live_grants(world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == client.id)
        .expect("the parent grant is live")
        .id
}

/// `GET /oauth2/authorize` for the one-off question, under `cookie`.
async fn authorize_one_off(world: &World, client: &Client, cookie: &str) -> AxumTestResponse {
    let details = one_off_details(world);
    let query = request_query(
        client,
        &fresh_state(),
        &[("authorization_details", details.as_str())],
    );
    AxumTestRequest::get(&format!("/oauth2/authorize?{query}"))
        .header("cookie", cookie)
        .send(app(&world.resources))
        .await
}

/// The athlete approves the step-up screen under `cookie`, with the digest
/// `screen` carried.
async fn approve_one_off(
    world: &World,
    client: &Client,
    cookie: &str,
    digest: &str,
) -> AxumTestResponse {
    let details = one_off_details(world);
    let token = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let challenge = challenge();
    let state = fresh_state();
    let form: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("decision", "approve"),
        ("csrf_token", token.as_str()),
        ("review_digest", digest),
        ("authorization_details", details.as_str()),
    ];
    AxumTestRequest::post("/oauth2/consent")
        .header("cookie", cookie)
        .form(&form)
        .send(app(&world.resources))
        .await
}

/// The step-up screen's review digest, the screen read under a fresh
/// hosted sign-in.
async fn step_up_digest(world: &World, client: &Client) -> String {
    let screen = authorize_one_off(world, client, &hosted_sign_in(world)).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    assert!(
        page.contains("asks for a one-off look at your health records"),
        "bug: the step-up screen is not shown: {page}"
    );
    assert!(
        !page.contains("standing_policy"),
        "bug: a step-up offered a standing policy: {page}"
    );
    hidden_field(&page, "review_digest").expect("the screen carries its digest")
}

/// A redirect's query parameter `name`.
fn param(response: &AxumTestResponse, name: &str) -> String {
    let location = response
        .header("location")
        .unwrap_or_else(|| panic!("a redirect: {}", response.body_text()));
    let absolute = if location.starts_with('/') {
        format!("https://base.test{location}")
    } else {
        location.to_owned()
    };
    Url::parse(&absolute)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .map_or_else(String::new, |(_, value)| value.into_owned())
}

/// The athlete's live step-up children.
async fn children(world: &World) -> Vec<ChildGrant> {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .list_live_child_grants(&world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap()
}

/// The events of grant `id`, oldest first.
async fn events(world: &World, id: &str) -> Vec<GrantEvent> {
    let mut receipts = world
        .resources
        .common
        .repos
        .oauth_grant_events
        .list_for_user(
            &world.tenant_id,
            &world.user.id.to_string(),
            Some(id),
            100,
            0,
        )
        .await
        .unwrap();
    // The store stamps whole seconds on SQLite: within one, the ending
    // comes last.
    receipts.sort_by_key(|receipt| (receipt.at, receipt.event.is_terminal()));
    receipts.into_iter().map(|receipt| receipt.event).collect()
}

/// A world whose athlete holds a training-coach parent for a client
/// declaring a T2 host, with the step-up screen lit, and a child approved
/// under it: the parent's and the child's ids.
async fn stepped_up() -> (World, Client, String, String) {
    let world = world().await;
    let client = register(&world).await;
    let parent = approve_parent(&world, &client).await;
    light_step_up(&world);
    declare_t2_host(&world, &client).await;
    let digest = step_up_digest(&world, &client).await;
    let approved = approve_one_off(&world, &client, &hosted_sign_in(&world), &digest).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    assert!(
        param(&approved, "error").is_empty(),
        "bug: the step-up was refused: {:?}",
        approved.header("location")
    );
    let code = code_of(&approved);
    let named = world
        .resources
        .common
        .repos
        .oauth2_server
        .peek_auth_code(&code, &client.id, REDIRECT, Timestamp::now())
        .await
        .unwrap()
        .expect("a live code")
        .client_grant_id;
    assert_eq!(
        named.as_deref(),
        Some(parent.as_str()),
        "bug: the step-up code does not name the parent"
    );
    let child = children(&world)
        .await
        .into_iter()
        .find(|child| child.terms.parent_grant_id == parent)
        .expect("bug: no child under the parent");
    (world, client, parent, child.grant.id)
}

// ============================================================================
// Dark until the catalogue words the screen
// ============================================================================

#[tokio::test]
async fn a_one_off_question_is_refused_before_sign_in_while_the_screen_is_dark() {
    let world = world().await;
    let client = register(&world).await;
    let refused = authorize_one_off(&world, &client, &world.cookie).await;
    assert_eq!(refused.status(), 303, "{}", refused.body_text());
    assert_eq!(
        param(&refused, "error"),
        "invalid_authorization_details",
        "bug: a dark step-up reached the athlete"
    );
    assert!(
        param(&refused, "error_description").contains("step-up is not offered yet"),
        "bug: {:?}",
        refused.header("location")
    );
}

// ============================================================================
// The fresh sign-in
// ============================================================================

#[tokio::test]
async fn the_web_apps_session_signs_in_again_before_a_step_up() {
    let world = world().await;
    let client = register(&world).await;
    approve_parent(&world, &client).await;
    light_step_up(&world);
    let screen = authorize_one_off(&world, &client, &world.cookie).await;
    assert_eq!(screen.status(), 303, "{}", screen.body_text());
    assert!(
        screen
            .header("location")
            .is_some_and(|location| location.starts_with("/oauth2/login")),
        "bug: the web app's session reached the step-up screen: {:?}",
        screen.header("location")
    );
}

#[tokio::test]
async fn a_step_up_posted_without_a_hosted_sign_in_is_refused() {
    let world = world().await;
    let client = register(&world).await;
    approve_parent(&world, &client).await;
    light_step_up(&world);
    declare_t2_host(&world, &client).await;
    let digest = step_up_digest(&world, &client).await;
    let refused = approve_one_off(&world, &client, &world.cookie, &digest).await;
    assert_eq!(
        param(&refused, "error"),
        "access_denied",
        "bug: the web app's cookie approved a step-up: {:?}",
        refused.header("location")
    );
    assert!(children(&world).await.is_empty(), "bug: a child was stored");
}

// ============================================================================
// The parent and the host
// ============================================================================

#[tokio::test]
async fn a_step_up_needs_a_live_grant_and_a_t2_host() {
    let world = world().await;
    let client = register(&world).await;
    light_step_up(&world);
    declare_t2_host(&world, &client).await;
    let digest = step_up_digest(&world, &client).await;
    let orphan = approve_one_off(&world, &client, &hosted_sign_in(&world), &digest).await;
    assert_eq!(param(&orphan, "error"), "access_denied", "bug: no parent");
    assert!(
        param(&orphan, "error_description").contains("step-up needs a live grant"),
        "bug: {:?}",
        orphan.header("location")
    );

    let undeclared = register(&world).await;
    approve_parent(&world, &undeclared).await;
    let digest = step_up_digest(&world, &undeclared).await;
    let refused = approve_one_off(&world, &undeclared, &hosted_sign_in(&world), &digest).await;
    assert_eq!(
        param(&refused, "error"),
        "access_denied",
        "bug: a client declaring no T2 host stepped up"
    );
    assert!(
        param(&refused, "error_description").contains("Host Profile T2"),
        "bug: {:?}",
        refused.header("location")
    );
    assert!(children(&world).await.is_empty(), "bug: a child was stored");
}

// ============================================================================
// The child
// ============================================================================

#[tokio::test]
async fn an_approved_step_up_is_a_child_the_parents_code_names() {
    let (world, _client, parent, child_id) = stepped_up().await;
    let child = children(&world)
        .await
        .into_iter()
        .find(|child| child.grant.id == child_id)
        .unwrap();
    assert_eq!(child.terms.acr, OWNER_VERIFIED_ACR, "bug: acr");
    assert_eq!(
        child.terms.host_profile_basis.as_str(),
        "declared_cimd",
        "bug: basis"
    );
    assert_eq!(child.terms.host_profile_tier, "T2", "bug: tier");
    assert!(
        child.grant.expires_at.is_some_and(|expires_at| {
            expires_at <= Timestamp::now() + dravr_tronc::dates::Elapsed::hours(1)
        }),
        "bug: a one-off child outlived its hour"
    );
    let details: Value =
        serde_json::from_str(child.grant.authorization_details.as_deref().unwrap()).unwrap();
    assert_eq!(
        details[0]["streams"],
        json!(["labs.summary"]),
        "bug: streams"
    );
    assert_eq!(details[0]["access_mode"], "single_use", "bug: access mode");
    assert_eq!(
        events(&world, &child_id).await,
        [GrantEvent::StepUp],
        "bug: receipt"
    );
    assert!(
        live_grants(&world)
            .await
            .iter()
            .all(|grant| grant.id != child_id),
        "bug: the child is listed as a grant of its own"
    );
    assert!(
        live_grants(&world)
            .await
            .iter()
            .any(|grant| grant.id == parent),
        "bug: the parent was edited or ended"
    );
    let standing = world
        .resources
        .common
        .repos
        .oauth2_server
        .client_grant_standing(
            &world.user.id.to_string(),
            &world.tenant_id,
            &child.grant.client_id,
        )
        .await
        .unwrap();
    let held = standing
        .grants
        .iter()
        .find(|grant| grant.id == parent)
        .expect("the parent stands");
    assert_eq!(
        held.child.as_ref().map(|child| child.id.as_str()),
        Some(child_id.as_str()),
        "bug: the standing does not carry the child under its parent"
    );
}

#[tokio::test]
async fn a_new_step_up_replaces_the_old_child() {
    let (world, client, parent, first) = stepped_up().await;
    let digest = step_up_digest(&world, &client).await;
    let again = approve_one_off(&world, &client, &hosted_sign_in(&world), &digest).await;
    assert!(
        param(&again, "error").is_empty(),
        "{:?}",
        again.header("location")
    );
    let live = children(&world).await;
    assert_eq!(live.len(), 1, "bug: two live children under one parent");
    assert_ne!(live[0].grant.id, first, "bug: the old child stayed live");
    assert_eq!(live[0].terms.parent_grant_id, parent, "bug: another parent");
    assert_eq!(
        events(&world, &first).await,
        [GrantEvent::StepUp, GrantEvent::Revoke],
        "bug: the replaced child's receipts"
    );
}

#[tokio::test]
async fn revoking_the_parent_ends_its_child_and_revoking_the_child_spares_the_parent() {
    let (world, client, parent, child) = stepped_up().await;
    let repo = &world.resources.common.repos.oauth2_server;
    let user = world.user.id.to_string();
    assert!(
        repo.revoke_client_grant(&child, &user, &world.tenant_id, GrantActor::User)
            .await
            .unwrap(),
        "bug: the child could not be revoked"
    );
    assert!(
        children(&world).await.is_empty(),
        "bug: the child stayed live"
    );
    assert!(
        live_grants(&world)
            .await
            .iter()
            .any(|grant| grant.id == parent),
        "bug: revoking the child ended its parent"
    );

    let digest = step_up_digest(&world, &client).await;
    let approved = approve_one_off(&world, &client, &hosted_sign_in(&world), &digest).await;
    assert!(
        param(&approved, "error").is_empty(),
        "{:?}",
        approved.header("location")
    );
    let second = children(&world).await[0].grant.id.clone();
    assert!(
        repo.revoke_client_grant(&parent, &user, &world.tenant_id, GrantActor::User)
            .await
            .unwrap(),
        "bug: the parent could not be revoked"
    );
    assert!(
        children(&world).await.is_empty(),
        "bug: a child outlived its parent"
    );
    assert_eq!(
        events(&world, &second).await,
        [GrantEvent::StepUp, GrantEvent::Revoke],
        "bug: the child's ending receipt"
    );
}

#[tokio::test]
async fn a_single_use_child_is_consumed_once_with_one_terminal_receipt() {
    let (world, _client, _parent, child) = stepped_up().await;
    let repo = &world.resources.common.repos.oauth2_server;
    let user = world.user.id.to_string();
    let (first, second) = tokio::join!(
        repo.consume_child_grant(&child, &user, &world.tenant_id),
        repo.consume_child_grant(&child, &user, &world.tenant_id),
    );
    let consumed = [first.unwrap(), second.unwrap()];
    assert_eq!(
        consumed.iter().filter(|consumed| **consumed).count(),
        1,
        "bug: a single-use child was consumed {consumed:?}"
    );
    assert!(
        !repo
            .consume_child_grant(&child, &user, &world.tenant_id)
            .await
            .unwrap(),
        "bug: a consumed child was consumed again"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp, GrantEvent::Consume],
        "bug: the consumed child's receipts"
    );
    assert!(
        children(&world).await.is_empty(),
        "bug: a consumed child stayed live"
    );
}
