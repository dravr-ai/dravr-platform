// ABOUTME: A DCR client's step-up on its own per-request Host Profile claim, and the owner's choice of streams on the step-up screen
// ABOUTME: The full path through /mcp, authorize, consent and back; refusals without the claim, without step-up, and for T3

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Step-up choices (owl phase 5, slice 4b; JF 2026-11-26)
//!
//! - **`declared_request`.** A client registered by DCR has no metadata
//!   document. It reaches T2 on its own per-request Host Profile claim, seen
//!   by `/mcp` on the call that asks for the step-up, and still only after a
//!   fresh sign-in on Dravr's page; never T3. The basis is recorded on the
//!   child, its receipt and Connected apps.
//! - **The owner's choice.** The step-up screen offers each requested stream
//!   as a checkbox, all ticked; the child holds only the ones left ticked,
//!   under a digest of that selection, and an unticked stream stays refused.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::hidden_field;
use crate::oauth2_grant_details_test::{
    access_detail, app, approve_with, challenge, code_of, constraints_detail, exchange,
    fresh_state, live_grants, register, tokens, world, Client, World, REDIRECT,
};
use crate::oauth2_step_up_test::{
    allow_health_records, approve_asking, authorize_asking, challenged_scope, children,
    connected_apps, file_a_lab, hosted_sign_in, light_step_up, param, read_records,
};

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{GrantEvent, HostProfileBasis, HostProfileClaim};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use serde_json::{json, Value};

/// A world whose athlete gave DCR client `client` (no metadata document) a
/// `preset` parent grant, with the step-up screen lit, a lab on file and
/// health records allowed to AI: the client, the parent's id and its token.
async fn dcr_parent(preset: &str) -> (World, Client, String, String) {
    let world = world().await;
    let client = register(&world).await;
    assert!(
        !client.id.starts_with("https://"),
        "the suite's client is a DCR registration"
    );
    light_step_up(&world);
    file_a_lab(&world).await;
    allow_health_records(&world).await;
    let approved = approve_with(&world, &client, &[], Some(preset)).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let (token, _) = tokens(&exchange(&world, &client, &code_of(&approved)).await);
    let parent = live_grants(&world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == client.id)
        .expect("the parent grant is live")
        .id;
    (world, client, parent, token)
}

/// `tools/call` of `get_health_records` for labs over `/mcp` under `bearer`,
/// declaring a Host Profile of `tier` when one is given: the status and the
/// `WWW-Authenticate` challenge.
async fn ask_labs(world: &World, bearer: &str, tier: Option<&str>) -> (u16, String) {
    let capabilities = tier.map_or_else(
        || json!({}),
        |tier| json!({"extensions": {"app.dravr.vault/host-profile": {"tier_max": tier, "version": 1}}}),
    );
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
                "arguments": {"kind": "labs"},
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": {"name": "dcr-host", "version": "1"},
                    "io.modelcontextprotocol/clientCapabilities": capabilities,
                },
            },
        }))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    (
        response.status(),
        response
            .header("www-authenticate")
            .unwrap_or_default()
            .to_owned(),
    )
}

/// The step-up screen for `scope` under a fresh hosted sign-in: the page.
async fn step_up_screen(world: &World, client: &Client, scope: &str) -> String {
    let screen = authorize_asking(world, client, &hosted_sign_in(world), &[("scope", scope)]).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    assert!(
        page.contains("asks for a one-off look at your health records"),
        "bug: not the step-up screen: {page}"
    );
    page
}

/// The athlete approves the step-up screen `page` showed for `scope`, under
/// `cookie`, its checkboxes as the screen renders them: the redirect.
async fn approve_screen(
    world: &World,
    client: &Client,
    cookie: &str,
    (page, scope): (&str, &str),
) -> AxumTestResponse {
    let digest = hidden_field(page, "review_digest").expect("the screen carries its digest");
    approve_asking(world, client, (cookie, &digest), &[("scope", scope)]).await
}

/// The signed payload of the `step_up` receipt of child `id`.
async fn step_up_receipt(world: &World, id: &str) -> Value {
    let receipt = world
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
        .unwrap()
        .into_iter()
        .find(|receipt| receipt.event == GrantEvent::StepUp)
        .expect("bug: the child has no step_up receipt");
    serde_json::from_str(&receipt.payload).unwrap()
}

// ============================================================================
// declared_request: a DCR client's own claim
// ============================================================================

#[tokio::test]
async fn a_dcr_clients_own_t2_claim_and_a_fresh_sign_in_open_t2_and_say_so() {
    common::init_server_config();
    let (world, client, parent, token) = dcr_parent("training_coach").await;

    // Its call declaring T2 on its own request is challenged.
    let (status, challenge) = ask_labs(&world, &token, Some("T2")).await;
    assert_eq!(
        status, 403,
        "bug: a DCR client's T2 claim was not challenged: {challenge}"
    );
    let scope = challenged_scope(&challenge);

    let page = step_up_screen(&world, &client, &scope).await;
    let approved = approve_screen(&world, &client, &hosted_sign_in(&world), (&page, &scope)).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    assert!(
        param(&approved, "error").is_empty(),
        "bug: the DCR client's step-up was refused: {:?}",
        approved.header("location")
    );

    let child = children(&world)
        .await
        .into_iter()
        .find(|child| child.terms.parent_grant_id == parent)
        .expect("bug: no child under the parent");
    assert_eq!(
        child.terms.host_profile_basis,
        HostProfileBasis::DeclaredRequest,
        "bug: the child does not record the per-request basis"
    );
    let receipt = step_up_receipt(&world, &child.grant.id).await;
    assert_eq!(
        receipt["host_profile"]["basis"], "declared_request",
        "bug: the receipt's basis: {receipt}"
    );
    let apps = connected_apps(&world).await;
    let app = apps
        .iter()
        .find(|app| app["id"] == parent.as_str())
        .expect("the parent is listed");
    assert_eq!(
        app["step_up"]["host_profile_basis"], "declared_request",
        "bug: Connected apps do not say the basis: {app}"
    );

    // The re-authorized token reads the lab while each call still claims T2.
    let (token, _) = tokens(&exchange(&world, &client, &code_of(&approved)).await);
    let unclaimed = read_records(&world, &token, "labs", None).await.to_string();
    assert!(
        !unclaimed.contains("ferritin"),
        "bug: a lab read without the request's claim: {unclaimed}"
    );
    let clinical = read_records(&world, &token, "clinical", Some("T2")).await;
    assert_eq!(
        clinical["result"]["isError"],
        json!(true),
        "bug: T3 reached a DCR client: {clinical}"
    );
    let labs = read_records(&world, &token, "labs", Some("T2"))
        .await
        .to_string();
    assert!(
        labs.contains("ferritin"),
        "bug: the T2 read did not open: {labs}"
    );
}

#[tokio::test]
async fn a_dcr_client_without_its_own_claim_is_never_stepped_up() {
    let (world, client, _parent, token) = dcr_parent("training_coach").await;

    // No claim, or one below T2: in band, no challenge, nothing recorded.
    for tier in [None, Some("T1")] {
        let (status, challenge) = ask_labs(&world, &token, tier).await;
        assert_eq!(status, 200, "bug: {tier:?} was challenged: {challenge}");
    }

    // The step-up asked anyway, with a fresh sign-in, is refused.
    let held = live_grants(&world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == client.id)
        .expect("the parent grant is live")
        .scope;
    let scope = format!("{held} pdpp:preset:one_off_question pdpp:stream:labs.summary");
    let page = step_up_screen(&world, &client, &scope).await;
    let approved = approve_screen(&world, &client, &hosted_sign_in(&world), (&page, &scope)).await;
    assert_eq!(
        param(&approved, "error"),
        "access_denied",
        "bug: a DCR client without a claim was stepped up: {:?}",
        approved.header("location")
    );
    assert!(children(&world).await.is_empty(), "bug: a child was stored");
}

#[tokio::test]
async fn a_dcr_clients_claim_needs_a_fresh_sign_in_and_goes_stale() {
    let (world, client, parent, token) = dcr_parent("training_coach").await;
    let (_, challenge) = ask_labs(&world, &token, Some("T2")).await;
    let scope = challenged_scope(&challenge);
    let page = step_up_screen(&world, &client, &scope).await;

    // The web app's own session never satisfies a step-up.
    let web_session = common::generate_test_token(&world.resources, &world.user).await;
    let approved = approve_screen(
        &world,
        &client,
        &format!("auth_token={web_session}"),
        (&page, &scope),
    )
    .await;
    assert!(
        children(&world).await.is_empty(),
        "bug: a step-up without a fresh hosted sign-in: {:?}",
        approved.header("location")
    );

    // A claim older than its window stands for nothing.
    let stale = HostProfileClaim {
        client_grant_id: parent,
        tier_max: "T2".to_owned(),
        seen_at: Timestamp::now() - Elapsed::seconds(16 * 60),
    };
    assert!(world
        .resources
        .common
        .repos
        .host_profile_claims
        .record_host_profile_claim(&world.user.id.to_string(), &stale)
        .await
        .unwrap());
    let approved = approve_screen(&world, &client, &hosted_sign_in(&world), (&page, &scope)).await;
    assert_eq!(
        param(&approved, "error"),
        "access_denied",
        "bug: a stale claim stood: {:?}",
        approved.header("location")
    );
    assert!(children(&world).await.is_empty(), "bug: a child was stored");
}

// ============================================================================
// The owner's choice of streams
// ============================================================================

/// A one-off question about labs and workouts.
fn labs_and_workouts(world: &World) -> String {
    json!([
        access_detail(
            world,
            &json!({
                "selection_preset": "one_off_question",
                "access_mode": "single_use",
                "streams": [
                    {"name": "health.vault.labs.summary", "necessity": "required"},
                    {"name": "health.vault.workout.summary", "necessity": "optional"},
                ],
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

/// `get_planned_workouts` over `/mcp` under `bearer`: the JSON-RPC body.
async fn planned_workouts(world: &World, bearer: &str) -> String {
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {bearer}"))
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", "tools/call")
        .header("mcp-name", "get_planned_workouts")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "get_planned_workouts",
                "arguments": {},
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {
                        "extensions": {"app.dravr.vault/host-profile": {"tier_max": "T2", "version": 1}}
                    },
                },
            },
        }))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.body_text()
}

/// Step `client` up for labs and workouts under a sleep-coach parent (which
/// holds no workouts), the claim seen first, posting `ticked` checkbox
/// fields: the redirect.
async fn choose(world: &World, client: &Client, token: &str, ticked: &[&str]) -> AxumTestResponse {
    let (status, _) = ask_labs(world, token, Some("T2")).await;
    assert_eq!(status, 403, "the claim is seen");
    let details = labs_and_workouts(world);
    let screen = authorize_asking(
        world,
        client,
        &hosted_sign_in(world),
        &[("authorization_details", &details)],
    )
    .await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    for field in [
        "step_up_stream:labs.summary",
        "step_up_stream:workout.summary",
    ] {
        assert!(
            page.contains(&format!(r#"name="{field}" value="on" checked"#)),
            "bug: the screen does not offer {field} ticked: {page}"
        );
    }
    let digest = hidden_field(&page, "review_digest").expect("the screen carries its digest");
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
        ("review_digest", digest.as_str()),
        ("authorization_details", details.as_str()),
    ];
    form.extend(ticked.iter().map(|field| (*field, "on")));
    AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &hosted_sign_in(world))
        .form(&form)
        .send(app(&world.resources))
        .await
}

#[tokio::test]
async fn an_unticked_stream_stays_out_of_the_child_and_refused() {
    common::init_server_config();
    let (world, client, parent, token) = dcr_parent("sleep_coach").await;
    let approved = choose(&world, &client, &token, &["step_up_stream:labs.summary"]).await;
    assert!(
        param(&approved, "error").is_empty(),
        "bug: the narrowed step-up was refused: {:?}",
        approved.header("location")
    );
    let child = children(&world)
        .await
        .into_iter()
        .find(|child| child.terms.parent_grant_id == parent)
        .expect("bug: no child under the parent");
    let details = child.grant.authorization_details.clone().unwrap();
    assert!(
        details.contains("labs.summary") && !details.contains("workout.summary"),
        "bug: the child holds an unticked stream: {details}"
    );
    let receipt = step_up_receipt(&world, &child.grant.id).await;
    let streams = receipt["grant"]["authorization_details"].to_string();
    assert!(
        streams.contains("labs.summary") && !streams.contains("workout.summary"),
        "bug: the receipt does not record the selection: {receipt}"
    );
    assert_eq!(
        receipt["consent_evidence"]["review_digest"].as_str(),
        child.grant.consent_version.as_deref(),
        "bug: the receipt's digest is not the child's"
    );
    let apps = connected_apps(&world).await;
    let app = apps
        .iter()
        .find(|app| app["id"] == parent.as_str())
        .unwrap();
    assert_eq!(
        app["step_up"]["streams"],
        json!(["labs.summary"]),
        "bug: Connected apps do not list the selection: {app}"
    );

    // Workouts, unticked, stay refused under the re-authorized token.
    let (token, _) = tokens(&exchange(&world, &client, &code_of(&approved)).await);
    let refused = planned_workouts(&world, &token).await;
    assert!(
        refused.contains("stream_not_granted"),
        "bug: an unticked stream was read: {refused}"
    );
}

#[tokio::test]
async fn every_stream_ticked_is_the_childs_and_the_digest_covers_the_selection() {
    let (world, client, parent, token) = dcr_parent("sleep_coach").await;
    let approved = choose(
        &world,
        &client,
        &token,
        &[
            "step_up_stream:labs.summary",
            "step_up_stream:workout.summary",
        ],
    )
    .await;
    assert!(
        param(&approved, "error").is_empty(),
        "{:?}",
        approved.header("location")
    );
    let all = children(&world)
        .await
        .into_iter()
        .find(|child| child.terms.parent_grant_id == parent)
        .unwrap();
    assert!(
        all.grant
            .authorization_details
            .as_deref()
            .is_some_and(|details| details.contains("workout.summary")),
        "bug: a ticked stream was dropped"
    );
    let (token, _) = tokens(&exchange(&world, &client, &code_of(&approved)).await);
    let read = planned_workouts(&world, &token).await;
    assert!(
        !read.contains("stream_not_granted"),
        "bug: a ticked stream was refused: {read}"
    );

    // The same request narrowed approves under another digest.
    let (world2, client2, parent2, token2) = dcr_parent("sleep_coach").await;
    let narrowed = choose(&world2, &client2, &token2, &["step_up_stream:labs.summary"]).await;
    assert!(
        param(&narrowed, "error").is_empty(),
        "{:?}",
        narrowed.header("location")
    );
    let one = children(&world2)
        .await
        .into_iter()
        .find(|child| child.terms.parent_grant_id == parent2)
        .unwrap();
    assert_ne!(
        one.grant.consent_version, all.grant.consent_version,
        "bug: the digest does not cover the selection"
    );
}

#[tokio::test]
async fn a_step_up_with_nothing_ticked_or_a_stream_not_asked_for_is_refused() {
    let (world, client, _parent, token) = dcr_parent("sleep_coach").await;
    let none = choose(&world, &client, &token, &[]).await;
    assert_eq!(
        param(&none, "error"),
        "access_denied",
        "bug: nothing ticked was approved: {:?}",
        none.header("location")
    );
    let stray = choose(
        &world,
        &client,
        &token,
        &[
            "step_up_stream:labs.summary",
            "step_up_stream:sleep.summary",
        ],
    )
    .await;
    assert_eq!(
        param(&stray, "error"),
        "access_denied",
        "bug: a stream not asked for was approved: {:?}",
        stray.header("location")
    );
    assert!(children(&world).await.is_empty(), "bug: a child was stored");
}
