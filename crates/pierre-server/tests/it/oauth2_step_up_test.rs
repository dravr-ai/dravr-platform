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

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::hidden_field;
use crate::oauth2_grant_details_test::{
    access_detail, app, approve_with, challenge, claimed_grant, code_of, constraints_detail,
    exchange, fresh_state, live_grants, register, request_query, tokens, world, Client, World,
    REDIRECT,
};

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_contremaitre::consent_strings::STEP_UP_SCREEN_KEYS;
use pierre_core::constants::oauth::providers::SENSITIVE_T2;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::constants::oauth_providers::NoticeKind;
use pierre_core::models::{
    ChildGrant, DataSource, DeviceType, GrantActor, GrantEvent, TenantId, OWNER_VERIFIED_ACR,
};
use pierre_database::repositories::{ClinicalRecordWrite, RecordFiling};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::routes::oauth_grants::OAuthGrantsRoutes;
use pierre_services::provider_notice::grant_consent;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

/// The step-up screen's lines, as a test words them: each test sets the
/// wording it reads rather than inheriting the pinned catalogue's.
const STEP_UP_LINES: [(&str, &str); 5] = [
    (
        "oauth.stepUp.title",
        "{0} asks for a one-off look at your health records",
    ),
    (
        "oauth.stepUp.t2Disclosure",
        "This includes your health records. {0} reads them once, then this access ends.",
    ),
    (
        "oauth.stepUp.host",
        "{0} says it handles health records with care. Dravr cannot check this.",
    ),
    ("oauth.stepUp.streamsTitle", "What {0} can read, once:"),
    ("oauth.stepUp.stream.labs", "Lab results (test wording)"),
];

/// Word the step-up screen in English, the athlete's language: every line
/// the screen can show, the ones this suite reads in its own wording.
fn light_step_up(world: &World) {
    let strings = &world.resources.mcp.messaging_strings_registry;
    let lines = STEP_UP_SCREEN_KEYS.iter().map(|key| {
        STEP_UP_LINES
            .iter()
            .find(|(own, _)| own == key)
            .map_or((*key, "a step-up line"), |line| *line)
    });
    for (key, line) in lines {
        let sha = format!("{:x}", Sha256::digest(line.as_bytes()));
        strings.update(key, "en", line.to_owned(), sha.clone());
        strings.update(key, &world.user.locale, line.to_owned(), sha);
    }
}

/// Unword the step-up screen in English, whatever the pinned catalogue says:
/// the dark launch's state.
fn darken_step_up(world: &World) {
    let strings = &world.resources.mcp.messaging_strings_registry;
    for key in STEP_UP_SCREEN_KEYS {
        strings.update(key, "en", String::new(), String::new());
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
    one_off_details_with(world, &Value::Null)
}

/// [`one_off_details`], its constraints narrowed to `budgets` (`null`:
/// none named).
fn one_off_details_with(world: &World, budgets: &Value) -> String {
    let mut constraints = json!({
        "max_tier": "T2",
        "step_up": {"required": true, "acr": "owner_verified"},
        "host_profile_required": "t2",
    });
    if !budgets.is_null() {
        constraints["budgets"] = budgets.clone();
    }
    json!([
        access_detail(
            world,
            &json!({
                "selection_preset": "one_off_question",
                "access_mode": "single_use",
                "streams": [{"name": "health.vault.labs.summary", "necessity": "required"}],
            })
        ),
        constraints_detail(&constraints),
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
    authorize_details(world, client, cookie, &one_off_details(world)).await
}

/// `GET /oauth2/authorize` for `details`, under `cookie`.
async fn authorize_details(
    world: &World,
    client: &Client,
    cookie: &str,
    details: &str,
) -> AxumTestResponse {
    authorize_asking(world, client, cookie, &[("authorization_details", details)]).await
}

/// `GET /oauth2/authorize` with `extra` parameters, under `cookie`.
async fn authorize_asking(
    world: &World,
    client: &Client,
    cookie: &str,
    extra: &[(&str, &str)],
) -> AxumTestResponse {
    let query = request_query(client, &fresh_state(), extra);
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
    approve_details(world, client, (cookie, digest), &one_off_details(world)).await
}

/// The athlete approves the step-up screen for `details` under `cookie`,
/// with the digest carried.
async fn approve_details(
    world: &World,
    client: &Client,
    (cookie, digest): (&str, &str),
    details: &str,
) -> AxumTestResponse {
    approve_asking(
        world,
        client,
        (cookie, digest),
        &[("authorization_details", details)],
    )
    .await
}

/// The athlete approves the screen a request with `extra` parameters
/// showed, under `cookie`, with the digest carried.
async fn approve_asking(
    world: &World,
    client: &Client,
    (cookie, digest): (&str, &str),
    extra: &[(&str, &str)],
) -> AxumTestResponse {
    let token = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let challenge = challenge();
    let state = fresh_state();
    let mut form: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("decision", "approve"),
        ("csrf_token", token.as_str()),
        ("review_digest", digest),
    ];
    form.extend_from_slice(extra);
    AxumTestRequest::post("/oauth2/consent")
        .header("cookie", cookie)
        .form(&form)
        .send(app(&world.resources))
        .await
}

/// The step-up screen's review digest, the screen read under a fresh
/// hosted sign-in.
async fn step_up_digest(world: &World, client: &Client) -> String {
    step_up_digest_of(world, client, &one_off_details(world)).await
}

/// [`step_up_digest`] of the screen for `details`.
async fn step_up_digest_of(world: &World, client: &Client, details: &str) -> String {
    let screen = authorize_details(world, client, &hosted_sign_in(world), details).await;
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
    assert!(
        page.contains("What client can read, once:") || page.contains("can read, once:"),
        "bug: the step-up screen does not head its streams: {page}"
    );
    assert!(
        page.contains("<li>Lab results (test wording)</li>"),
        "bug: the step-up screen does not list the streams it reads: {page}"
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
    let (world, client, parent, child, _code) = stepped_up_with_code().await;
    (world, client, parent, child)
}

/// [`stepped_up`], with the code the step-up approval returned.
async fn stepped_up_with_code() -> (World, Client, String, String, String) {
    stepped_up_under(&Value::Null).await
}

/// [`stepped_up_with_code`], the child's constraints narrowed to `budgets`.
async fn stepped_up_under(budgets: &Value) -> (World, Client, String, String, String) {
    let world = world().await;
    let client = register(&world).await;
    let parent = approve_parent(&world, &client).await;
    light_step_up(&world);
    declare_t2_host(&world, &client).await;
    let details = one_off_details_with(&world, budgets);
    let digest = step_up_digest_of(&world, &client, &details).await;
    let approved = approve_details(
        &world,
        &client,
        (&hosted_sign_in(&world), &digest),
        &details,
    )
    .await;
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
    (world, client, parent, child.grant.id, code)
}

// ============================================================================
// Dark until the catalogue words the screen
// ============================================================================

#[tokio::test]
async fn a_one_off_question_is_refused_before_sign_in_while_the_screen_is_dark() {
    let world = world().await;
    darken_step_up(&world);
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
        child
            .grant
            .expires_at
            .is_some_and(|expires_at| { expires_at <= Timestamp::now() + Elapsed::hours(1) }),
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

#[tokio::test]
async fn a_step_up_under_a_plain_grant_is_refused() {
    let world = world().await;
    let client = register(&world).await;
    let plain = approve_with(&world, &client, &[], None).await;
    assert_eq!(plain.status(), 303, "{}", plain.body_text());
    light_step_up(&world);
    declare_t2_host(&world, &client).await;
    let digest = step_up_digest(&world, &client).await;
    let refused = approve_one_off(&world, &client, &hosted_sign_in(&world), &digest).await;
    assert_eq!(
        param(&refused, "error"),
        "access_denied",
        "bug: a child of a plain grant, which no ceiling governs, was stored"
    );
    assert!(
        param(&refused, "error_description").contains("preset"),
        "bug: {:?}",
        refused.header("location")
    );
    assert!(children(&world).await.is_empty(), "bug: a child was stored");
}

// ============================================================================
// T2 end to end: the child's lab read over /mcp
// ============================================================================

/// The athlete's lab result, filed three days ago.
async fn file_a_lab(world: &World) {
    let repos = &world.resources.common.repos;
    let tenant = TenantId::from_uuid(world.tenant_id.parse().unwrap());
    let source = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: world.user.id.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let filing = RecordFiling {
        tenant_id: &tenant,
        user_id: world.user.id,
        provider: INTERVALS_ICU,
        data_source_id: &source,
    };
    let lab = ClinicalRecordWrite {
        record_id: "lab-ferritin".to_owned(),
        resource_type: "Observation".to_owned(),
        tier: "T2".to_owned(),
        sensitivity_class: None,
        fhir_version: "4.0.1".to_owned(),
        resource: json!({ "resourceType": "Observation", "id": "ferritin" }).to_string(),
        stripped: json!({ "resourceType": "Observation", "id": "ferritin" }).to_string(),
        provenance: json!({ "resourceType": "Provenance" }).to_string(),
        flat: None,
        derivation: "verbatim".to_owned(),
        review_state: "none".to_owned(),
        checks: "[]".to_owned(),
        locator: None,
        read_by: None,
        effective_at: Some(Timestamp::now() - Elapsed::days(3)),
        lab_document_id: None,
        legacy_id: None,
    };
    repos
        .health_records
        .upsert_clinical_records(filing, &[lab])
        .await
        .unwrap();
}

/// The athlete lets AI read their health records (`sensitive_t2`).
async fn allow_health_records(world: &World) {
    grant_consent(
        &world.resources.common.repos,
        Uuid::parse_str(&world.tenant_id).unwrap(),
        world.user.id,
        SENSITIVE_T2,
        NoticeKind::AiConsent,
        "Health records",
    )
    .await
    .unwrap();
}

/// `get_health_records` of `kind` over `/mcp` under `bearer`, the request
/// declaring a Host Profile of `tier` when one is given: the JSON-RPC body.
async fn read_records(world: &World, bearer: &str, kind: &str, tier: Option<&str>) -> Value {
    let capabilities = tier.map_or_else(
        || json!({}),
        |tier| {
            json!({ "extensions": {
                "app.dravr.vault/host-profile": { "tier_max": tier, "version": 1 }
            }})
        },
    );
    let start = (Timestamp::now() - Elapsed::days(30)).to_rfc3339();
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {bearer}"))
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", "tools/call")
        .header("mcp-name", "get_health_records")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "get_health_records",
                "arguments": { "kind": kind, "start": start },
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": capabilities,
                },
            },
        }))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    assert_eq!(response.status(), 200, "{kind}: {}", response.body_text());
    serde_json::from_str(&response.body_text()).unwrap()
}

#[tokio::test]
async fn the_childs_lab_read_reaches_an_outside_client_once_and_nothing_else_does() {
    common::init_server_config();
    let (world, client, parent, child, code) = stepped_up_with_code().await;
    file_a_lab(&world).await;
    allow_health_records(&world).await;

    // The token the step-up's code mints names the parent, never the child.
    let (token, _) = tokens(&exchange(&world, &client, &code).await);
    assert_eq!(
        claimed_grant(&token).as_deref(),
        Some(parent.as_str()),
        "bug: the re-authorized token does not name the parent"
    );

    // Without the request's own Host Profile claim, T2 stays shut.
    let unclaimed = read_records(&world, &token, "labs", None).await.to_string();
    assert!(
        !unclaimed.contains("ferritin"),
        "bug: a lab read without the request's Host Profile claim: {unclaimed}"
    );
    // Another kind declares T3 streams: refused under any child.
    let clinical = read_records(&world, &token, "clinical", Some("T2")).await;
    assert_eq!(
        clinical["result"]["isError"],
        json!(true),
        "bug: a clinical read reached an outside client: {clinical}"
    );
    assert!(
        !clinical.to_string().contains("step_up_required"),
        "bug: a step-up was offered for T3 streams: {clinical}"
    );
    assert_eq!(
        children(&world).await.len(),
        1,
        "bug: a refusal consumed the child"
    );

    // The labs, read once under the child.
    let labs = read_records(&world, &token, "labs", Some("T2")).await;
    assert!(
        labs.to_string().contains("ferritin"),
        "bug: the child's lab read was not served: {labs}"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp, GrantEvent::Consume],
        "bug: the single-use read did not consume the child"
    );
    let again = read_records(&world, &token, "labs", Some("T2"))
        .await
        .to_string();
    assert!(
        !again.contains("ferritin"),
        "bug: a consumed single-use child read again: {again}"
    );
}

#[tokio::test]
async fn two_lab_reads_arriving_together_under_a_single_use_child_serve_one() {
    common::init_server_config();
    let (world, client, _parent, child, code) = stepped_up_with_code().await;
    file_a_lab(&world).await;
    allow_health_records(&world).await;
    let (token, _) = tokens(&exchange(&world, &client, &code).await);
    let (first, second) = tokio::join!(
        read_records(&world, &token, "labs", Some("T2")),
        read_records(&world, &token, "labs", Some("T2")),
    );
    let served: Vec<bool> = [&first, &second]
        .iter()
        .map(|body| body.to_string().contains("ferritin"))
        .collect();
    assert_eq!(
        served.iter().filter(|served| **served).count(),
        1,
        "bug: a single-use child served {served:?}: {first} / {second}"
    );
    let refused = if served[0] { &second } else { &first };
    assert!(
        refused.to_string().contains("grant_consumed"),
        "bug: the second read was not refused as consumed: {refused}"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp, GrantEvent::Consume],
        "bug: the child's receipts"
    );
}

#[tokio::test]
async fn a_step_up_childs_requests_an_hour_are_its_own() {
    common::init_server_config();
    let (world, client, _parent, child, code) =
        stepped_up_under(&json!({ "requests_per_hour": 1 })).await;
    file_a_lab(&world).await;
    let (token, _) = tokens(&exchange(&world, &client, &code).await);

    // Under the child, refused by the athlete's switch: its one request.
    let unconsented = read_records(&world, &token, "labs", Some("T2")).await;
    assert!(
        !unconsented.to_string().contains("ferritin"),
        "bug: a lab read without sensitive_t2: {unconsented}"
    );
    allow_health_records(&world).await;
    let over = read_records(&world, &token, "labs", Some("T2"))
        .await
        .to_string();
    assert!(
        over.contains("budget_exhausted") && !over.contains("ferritin"),
        "bug: the child's second request this hour was admitted: {over}"
    );
    assert_eq!(
        events(&world, &child).await,
        [GrantEvent::StepUp],
        "bug: a refused call consumed the child"
    );
}

// ============================================================================
// The 403 challenge and the host's re-authorization (owl §7.6 row 2)
// ============================================================================

/// `tools/call` of `get_health_records` for `kind` over `/mcp` under
/// `bearer`: the status, the `WWW-Authenticate` header and the body.
async fn records_status(world: &World, bearer: &str, kind: &str) -> (u16, String, String) {
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {bearer}"))
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", "tools/call")
        .header("mcp-name", "get_health_records")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "get_health_records", "arguments": {"kind": kind}},
        }))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    (
        response.status(),
        response
            .header("www-authenticate")
            .unwrap_or_default()
            .to_owned(),
        response.body_text(),
    )
}

/// A world whose athlete holds a training-coach parent for a T2 host, with
/// the step-up screen lit: the client, the parent's id and its token.
async fn under_a_parent() -> (World, Client, String, String) {
    let world = world().await;
    let client = register(&world).await;
    light_step_up(&world);
    declare_t2_host(&world, &client).await;
    let approved = approve_with(&world, &client, &[], Some("training_coach")).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let code = code_of(&approved);
    let (token, _) = tokens(&exchange(&world, &client, &code).await);
    let parent = live_grants(&world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == client.id)
        .expect("the parent grant is live")
        .id;
    (world, client, parent, token)
}

/// The `scope` a `WWW-Authenticate` challenge names.
fn challenged_scope(challenge: &str) -> String {
    challenge
        .split("scope=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("a scope in {challenge}"))
        .to_owned()
}

#[tokio::test]
async fn a_call_a_step_up_would_cure_is_answered_403_with_the_cures_scope() {
    let (world, _client, _parent, token) = under_a_parent().await;
    let (status, challenge, body) = records_status(&world, &token, "labs").await;
    assert_eq!(
        status, 403,
        "bug: a curable call was not challenged: {body}"
    );
    assert!(
        challenge.contains("error=\"insufficient_scope\""),
        "bug: {challenge}"
    );
    let scope = challenged_scope(&challenge);
    assert!(
        scope.ends_with("pdpp:preset:one_off_question pdpp:stream:labs.summary"),
        "bug: the challenge's scope: {scope}"
    );
    assert!(
        scope.contains("fitness:read"),
        "bug: the challenge drops the token's own grant: {scope}"
    );

    // A call no step-up cures (T3 streams beside) stays in band.
    let (status, challenge, body) = records_status(&world, &token, "clinical").await;
    assert_eq!(
        status, 200,
        "bug: an incurable call was challenged: {challenge}"
    );
    assert!(
        !body.contains("step_up_required"),
        "bug: a step-up was offered for T3 streams: {body}"
    );

    // Dark, nothing is challenged.
    darken_step_up(&world);
    let (status, _, _) = records_status(&world, &token, "labs").await;
    assert_eq!(status, 200, "bug: a dark step-up was challenged");
}

#[tokio::test]
async fn a_host_reauthorizing_with_the_challenge_gets_a_child_its_token_names_the_parent_of() {
    let (world, client, parent, token) = under_a_parent().await;
    let (_, challenge, _) = records_status(&world, &token, "labs").await;
    let scope = challenged_scope(&challenge);

    let screen = authorize_asking(
        &world,
        &client,
        &hosted_sign_in(&world),
        &[("scope", &scope)],
    )
    .await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    assert!(
        page.contains("asks for a one-off look at your health records")
            && page.contains("<li>Lab results (test wording)</li>"),
        "bug: the challenge's scope is not the step-up screen: {page}"
    );
    let digest = hidden_field(&page, "review_digest").expect("the screen carries its digest");
    let approved = approve_asking(
        &world,
        &client,
        (&hosted_sign_in(&world), &digest),
        &[("scope", &scope)],
    )
    .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    assert!(
        param(&approved, "error").is_empty(),
        "bug: the re-authorization was refused: {:?}",
        approved.header("location")
    );
    let child = children(&world).await;
    assert_eq!(child.len(), 1, "bug: no child under the parent");
    assert_eq!(
        child[0].terms.parent_grant_id, parent,
        "bug: another parent"
    );
    let response = exchange(&world, &client, &code_of(&approved)).await;
    let (stepped, _) = tokens(&response);
    assert_eq!(
        claimed_grant(&stepped).as_deref(),
        Some(parent.as_str()),
        "bug: the re-authorized token does not name the parent"
    );
    // The token response states the child beside the parent it names.
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    let step_up = &body["step_up"];
    assert_eq!(
        step_up["grant_id"],
        json!(child[0].grant.id),
        "bug: the token response does not state the child: {body}"
    );
    assert_eq!(
        step_up["step_up_result"]["acr"],
        json!(OWNER_VERIFIED_ACR),
        "bug: {step_up}"
    );
    assert!(
        step_up["step_up_result"]["device"].is_null(),
        "bug: a device named: {step_up}"
    );
    assert_eq!(
        step_up["host_profile"],
        json!({"basis": "declared_cimd", "tier": "T2"}),
        "bug: {step_up}"
    );
    assert_eq!(
        step_up["authorization_details"][0]["selection_preset"],
        json!("one_off_question"),
        "bug: {step_up}"
    );
}

// ============================================================================
// Connected apps
// ============================================================================

/// The athlete's Connected apps, as the web and mobile lists read them.
async fn connected_apps(world: &World) -> Vec<Value> {
    let session = common::generate_test_token(&world.resources, &world.user).await;
    let listed: Value = AxumTestRequest::get("/api/me/oauth-grants")
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await
        .json();
    listed["grants"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing: {listed}"))
        .clone()
}

#[tokio::test]
async fn connected_apps_list_the_child_under_its_app_and_revoke_it_alone() {
    let (world, _client, parent, child) = stepped_up().await;
    let apps = connected_apps(&world).await;
    assert_eq!(
        apps.len(),
        1,
        "bug: the child is listed as an app: {apps:?}"
    );
    assert_eq!(apps[0]["id"], json!(parent), "bug: {apps:?}");
    assert_eq!(
        apps[0]["step_up"]["id"],
        json!(child),
        "bug: the child is not under its app: {apps:?}"
    );
    assert!(
        apps[0]["step_up"]["expires_at"].is_string(),
        "bug: the child's end is not listed: {apps:?}"
    );

    let session = common::generate_test_token(&world.resources, &world.user).await;
    let revoked = AxumTestRequest::delete(&format!("/api/me/oauth-grants/{child}"))
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await;
    assert_eq!(revoked.status(), 204, "bug: {}", revoked.body_text());
    let apps = connected_apps(&world).await;
    assert_eq!(apps.len(), 1, "bug: revoking the child ended its app");
    assert!(
        apps[0]["step_up"].is_null(),
        "bug: a revoked child is still listed: {apps:?}"
    );
}
