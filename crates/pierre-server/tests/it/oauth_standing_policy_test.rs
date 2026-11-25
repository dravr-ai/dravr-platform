// ABOUTME: Standing policies renew a preset grant without the consent screen, and only on their own terms
// ABOUTME: Created by the consent screen's box or seeded through the repository, driven through the real authorize, consent and routes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Owl phase 5 slice 3b, item 5 (carnet#1544, design row 35, owl §6.4).
//!
//! The consent screen's box beside a preset stores a policy with the
//! approved grant's terms, digest and language; the other tests seed one
//! through the repository from such a grant. The renewal answers where the
//! screen would be skipped, records a grant citing the policy as the AS, and
//! that grant's own skip answers the next authorization. Other terms, an
//! admin, a client returning to a local program, a changed consent text, two
//! policies, a removed policy, a revoked grant (which removes its client's
//! policies) and a deleted client all show the screen again.

use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::consent_form::{authorize_query, hidden_field};
use crate::oauth2_grant_details_test::{
    access_detail, approve_with, authorize_query_as, challenge, code_of, constraints_detail,
    forget_standing, fresh_state, live_grants, register, request_query, shown, world, Client,
    World,
};

use std::sync::Arc;

use axum::http::StatusCode;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::oauth2_server::authorization_details::{preset_defaults, Preset, ResolvedDetails};
use pierre_contremaitre::consent_strings::{KEY_CONSENT_EXPRESS, KEY_CONSENT_STANDING_POLICY};
use pierre_core::models::{
    GrantActor, GrantEvent, OAuth2Client, OAuthClientGrant, OAuthGrantEvent, OAuthStandingPolicy,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::routes::oauth_grants::OAuthGrantsRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

const EXPIRE_SQL: &str = "UPDATE oauth_client_grants SET expires_at = $1 WHERE id = $2";

/// The athlete approves `client` on the picker for `preset`: the grant.
async fn approve_preset(world: &World, client: &Client, preset: &str) -> OAuthClientGrant {
    let approved = approve_with(world, client, &[], Some(preset)).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    live_grant(world, client).await.expect("the preset grant")
}

/// The athlete's live grant to `client`, if any.
async fn live_grant(world: &World, client: &Client) -> Option<OAuthClientGrant> {
    live_grants(world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == client.id && grant.is_live_at(Timestamp::now()))
}

/// The standing policy the athlete would have ticked beside `grant`: its
/// preset, terms, digest and language.
async fn seed_policy(world: &World, grant: &OAuthClientGrant) -> OAuthStandingPolicy {
    let details =
        ResolvedDetails::from_document(grant.authorization_details.as_deref().unwrap()).unwrap();
    let policy = OAuthStandingPolicy {
        id: Uuid::new_v4().to_string(),
        tenant_id: world.tenant_id.clone(),
        user_id: world.user.id.to_string(),
        client_id: grant.client_id.clone(),
        preset: details.preset().name().to_owned(),
        scope: grant.scope.clone(),
        authorization_details: details.terms().to_document().unwrap(),
        consent_version: grant.consent_version.clone().unwrap(),
        consent_locale: grant.consent_locale.clone().unwrap(),
        created_at: Timestamp::now(),
        revoked_at: None,
    };
    world
        .resources
        .common
        .repos
        .oauth_standing_policies
        .store_standing_policy(&policy)
        .await
        .unwrap();
    policy
}

/// Move `grant_id`'s expiry into the past, as 30 days do.
async fn expire(world: &World, grant_id: &str) {
    let past = Timestamp::now() - Elapsed::minutes(1);
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(EXPIRE_SQL)
                .bind(past)
                .bind(grant_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(EXPIRE_SQL)
                .bind(past)
                .bind(grant_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// An athlete holding a training-coach grant to `client` and the policy
/// ticked beside it, the grant now expired.
async fn expired_under_policy(world: &World, client: &Client) -> (String, OAuthStandingPolicy) {
    let grant = approve_preset(world, client, "training_coach").await;
    let policy = seed_policy(world, &grant).await;
    expire(world, &grant.id).await;
    forget_standing(world, client).await;
    (grant.id, policy)
}

/// A scope-only authorization of `client`: its status.
async fn authorize(world: &World, client: &Client) -> u16 {
    authorize_query_as(world, &request_query(client, &fresh_state(), &[]), true)
        .await
        .status()
}

/// The receipts of `grant_id`, newest first.
async fn receipts(world: &World, grant_id: &str) -> Vec<OAuthGrantEvent> {
    world
        .resources
        .common
        .repos
        .oauth_grant_events
        .list_for_user(
            &world.tenant_id,
            &world.user.id.to_string(),
            Some(grant_id),
            100,
            0,
        )
        .await
        .unwrap()
}

/// The athlete's live policies.
async fn policies(world: &World) -> Vec<OAuthStandingPolicy> {
    world
        .resources
        .common
        .repos
        .oauth_standing_policies
        .list_standing_policies(&world.tenant_id, &world.user.id.to_string(), None)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_policy_renews_its_expired_preset_grant_as_the_as_and_the_renewal_skips_itself() {
    let world = world().await;
    let client = register(&world).await;
    let (expired, policy) = expired_under_policy(&world, &client).await;

    let renewed =
        authorize_query_as(&world, &request_query(&client, &fresh_state(), &[]), true).await;
    assert_eq!(
        renewed.status(),
        303,
        "bug: the policy did not renew: {}",
        renewed.body_text()
    );
    code_of(&renewed);
    let grant = live_grant(&world, &client)
        .await
        .expect("bug: no renewed grant");
    assert_ne!(grant.id, expired, "bug: the expired row was not replaced");
    // Replacing the expired grant is not a revoke: the policy stands.
    assert_eq!(
        policies(&world)
            .await
            .iter()
            .map(|live| live.id.as_str())
            .collect::<Vec<_>>(),
        vec![policy.id.as_str()],
        "bug: the renewal ended the policy that renewed it"
    );
    assert_eq!(
        (
            grant.consent_version.as_deref(),
            grant.consent_locale.as_deref()
        ),
        (
            Some(policy.consent_version.as_str()),
            Some(policy.consent_locale.as_str())
        ),
        "bug: the renewal is not recorded under the policy's digest and language"
    );
    let details =
        ResolvedDetails::from_document(grant.authorization_details.as_deref().unwrap()).unwrap();
    let terms = ResolvedDetails::from_document(&policy.authorization_details).unwrap();
    assert!(
        details.same_terms(&terms) && details.since().is_some(),
        "bug: the renewal is not the policy's terms frozen now"
    );

    // The expired grant ends as expired; the renewal's issue cites the policy.
    let ending = receipts(&world, &expired)
        .await
        .into_iter()
        .find(|receipt| receipt.event != GrantEvent::Issue)
        .expect("bug: the expired grant did not end");
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Expire, GrantActor::Auto),
        "bug: the replaced grant did not end as expired"
    );
    let issues = receipts(&world, &grant.id).await;
    assert_eq!(issues.len(), 1, "bug: {} receipts", issues.len());
    assert_eq!(
        issues[0].actor,
        GrantActor::Auto,
        "bug: a renewal is the athlete's"
    );
    let payload: Value = serde_json::from_str(&issues[0].payload).unwrap();
    assert_eq!(
        payload["approval"]["standing_policy_ref"],
        json!({"id": policy.id, "consent_version": policy.consent_version}),
        "bug: the renewal's receipt does not cite the policy"
    );

    // The renewed grant answers the next authorization by itself.
    assert_eq!(
        authorize(&world, &client).await,
        303,
        "bug: the renewal's own skip"
    );
    let again = live_grant(&world, &client).await.unwrap();
    assert_eq!(
        again.id, grant.id,
        "bug: the next authorization renewed again"
    );
    assert_eq!(
        receipts(&world, &grant.id).await.len(),
        1,
        "bug: a skip wrote a receipt"
    );
}

#[tokio::test]
async fn a_request_naming_the_policys_own_details_is_renewed() {
    let world = world().await;
    let client = register(&world).await;
    let details = json!([
        access_detail(&world, &json!({})),
        constraints_detail(&json!({"budgets": {"requests_per_hour": 7}}))
    ])
    .to_string();
    let extra = [("authorization_details", details.as_str())];
    let approved = approve_with(&world, &client, &extra, None).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let grant = live_grant(&world, &client)
        .await
        .expect("the details grant");
    let policy = seed_policy(&world, &grant).await;
    expire(&world, &grant.id).await;
    forget_standing(&world, &client).await;

    let renewed = authorize_query_as(
        &world,
        &request_query(&client, &fresh_state(), &extra),
        true,
    )
    .await;
    assert_eq!(
        renewed.status(),
        303,
        "bug: a request for the policy's own details was not renewed: {}",
        renewed.body_text()
    );
    let renewal = live_grant(&world, &client).await.expect("bug: no renewal");
    let issue = receipts(&world, &renewal.id).await;
    let payload: Value = serde_json::from_str(&issue[0].payload).unwrap();
    assert_eq!(
        (
            issue[0].actor,
            &payload["approval"]["standing_policy_ref"]["id"]
        ),
        (GrantActor::Auto, &Value::from(policy.id)),
        "bug: the details renewal does not cite its policy"
    );
}

#[tokio::test]
async fn a_request_naming_other_terms_shows_the_screen() {
    let world = world().await;
    let client = register(&world).await;
    expired_under_policy(&world, &client).await;
    let narrowed = json!([
        access_detail(&world, &json!({})),
        constraints_detail(&json!({"budgets": {"requests_per_hour": 7}}))
    ])
    .to_string();
    let screen = authorize_query_as(
        &world,
        &request_query(
            &client,
            &fresh_state(),
            &[("authorization_details", narrowed.as_str())],
        ),
        true,
    )
    .await;
    assert_eq!(
        screen.status(),
        200,
        "bug: a request with other terms renewed"
    );
}

#[tokio::test]
async fn raw_streams_outside_a_preset_are_refused_before_any_policy_is_read() {
    let world = world().await;
    let client = register(&world).await;
    expired_under_policy(&world, &client).await;
    let raw = json!([
        access_detail(&world, &json!({})),
        constraints_detail(&json!({"raw_streams": ["health.vault.workout.raw"]}))
    ])
    .to_string();
    let refused = authorize_query_as(
        &world,
        &request_query(
            &client,
            &fresh_state(),
            &[("authorization_details", raw.as_str())],
        ),
        true,
    )
    .await;
    let location = refused.header("location").unwrap_or_default().to_owned();
    assert!(
        location.contains("error=invalid_authorization_details") && !location.contains("code="),
        "bug: a raw-stream request was not refused: {location}"
    );
    assert!(
        live_grant(&world, &client).await.is_none(),
        "bug: a refused request recorded a grant"
    );
}

#[tokio::test]
async fn an_admin_always_sees_the_screen() {
    let world = world().await;
    let client = register(&world).await;
    expired_under_policy(&world, &client).await;
    world
        .resources
        .common
        .repos
        .users
        .set_admin_status(world.user.id, true)
        .await
        .unwrap();
    assert_eq!(
        authorize(&world, &client).await,
        200,
        "bug: an admin's grant was renewed"
    );
}

#[tokio::test]
async fn a_changed_consent_text_shows_the_screen() {
    let world = world().await;
    let client = register(&world).await;
    let (_, policy) = expired_under_policy(&world, &client).await;
    world.resources.mcp.messaging_strings_registry.update(
        KEY_CONSENT_EXPRESS,
        &policy.consent_locale,
        "A reworded express consent.".to_owned(),
        format!("reworded-{}", Uuid::new_v4().simple()),
    );
    assert_eq!(
        authorize(&world, &client).await,
        200,
        "bug: a policy ticked under an older text renewed"
    );
}

#[tokio::test]
async fn two_policies_for_one_client_show_the_screen() {
    let world = world().await;
    let client = register(&world).await;
    // Both presets approved on the screen in turn, each with its box.
    let training = approve_preset(&world, &client, "training_coach").await;
    expire(&world, &training.id).await;
    forget_standing(&world, &client).await;
    let sleep = approve_preset(&world, &client, "sleep_coach").await;
    seed_policy(&world, &training).await;
    seed_policy(&world, &sleep).await;
    expire(&world, &sleep.id).await;
    forget_standing(&world, &client).await;
    assert_eq!(policies(&world).await.len(), 2, "fixture: two policies");
    assert_eq!(
        authorize(&world, &client).await,
        200,
        "bug: the AS picked a preset between two policies"
    );
}

#[tokio::test]
async fn a_client_returning_to_a_local_program_shows_the_screen() {
    let world = world().await;
    let loopback = "http://127.0.0.1/callback";
    let client_id = format!("local-{}", Uuid::new_v4().simple());
    let repos = &world.resources.common.repos;
    repos
        .oauth2_server
        .store_client(&OAuth2Client {
            id: Uuid::new_v4().to_string(),
            client_id: client_id.clone(),
            client_secret_hash: String::new(),
            redirect_uris: vec![loopback.to_owned()],
            grant_types: vec!["authorization_code".to_owned()],
            response_types: vec!["code".to_owned()],
            client_name: Some("Local program".to_owned()),
            client_uri: None,
            scope: None,
            created_at: Timestamp::now(),
            expires_at: None,
        })
        .await
        .unwrap();
    let challenge = challenge();
    let query = || {
        authorize_query(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", loopback),
            ("state", fresh_state().as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ])
    };
    // The policy holds the digest this client's own screen carries for the
    // preset, so its terms and text would renew: only the gate stands between.
    let screen = authorize_query_as(&world, &query(), true).await;
    assert_eq!(screen.status(), 200, "fixture: the screen is shown");
    let digest = hidden_field(&screen.body_text(), "review_digest_training_coach")
        .expect("fixture: the picker carries the preset's digest");
    let scope = hidden_field(&screen.body_text(), "scope")
        .expect("fixture: the screen carries the scope it grants");
    repos
        .oauth_standing_policies
        .store_standing_policy(&OAuthStandingPolicy {
            id: Uuid::new_v4().to_string(),
            tenant_id: world.tenant_id.clone(),
            user_id: world.user.id.to_string(),
            client_id: client_id.clone(),
            preset: "training_coach".to_owned(),
            scope,
            authorization_details: preset_terms(&world),
            consent_version: digest,
            consent_locale: world.user.locale.clone(),
            created_at: Timestamp::now(),
            revoked_at: None,
        })
        .await
        .unwrap();
    let again = authorize_query_as(&world, &query(), true).await;
    assert_eq!(
        again.status(),
        200,
        "bug: a client returning to a local program was renewed: {:?}",
        again.header("location")
    );
}

/// The training-coach terms a picker grant on this server holds.
fn preset_terms(world: &World) -> String {
    let source = world.resources.common.config.oauth2_server.mcp_resources()[0].to_owned();
    preset_defaults(Preset::TrainingCoach, &source)
        .to_document()
        .unwrap()
}

#[tokio::test]
async fn revoking_a_grant_removes_its_clients_policies_so_nothing_renews_it() {
    let world = world().await;
    let client = register(&world).await;
    let grant = approve_preset(&world, &client, "training_coach").await;
    seed_policy(&world, &grant).await;
    let other_client = register(&world).await;
    let other_grant = approve_preset(&world, &other_client, "sleep_coach").await;
    let other_policy = seed_policy(&world, &other_grant).await;
    let session = common::generate_test_token(&world.resources, &world.user).await;

    AxumTestRequest::delete(&format!("/api/me/oauth-grants/{}", grant.id))
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    assert_eq!(
        policies(&world)
            .await
            .iter()
            .map(|policy| policy.id.as_str())
            .collect::<Vec<_>>(),
        vec![other_policy.id.as_str()],
        "bug: the revoked client's policy stands, or another client's went with it"
    );
    forget_standing(&world, &client).await;
    assert_eq!(
        authorize(&world, &client).await,
        200,
        "bug: a policy renewed the grant the athlete revoked"
    );
}

#[tokio::test]
async fn removing_a_policy_stops_renewals_and_leaves_its_grants() {
    let world = world().await;
    let client = register(&world).await;
    let grant = approve_preset(&world, &client, "training_coach").await;
    let policy = seed_policy(&world, &grant).await;
    let session = common::generate_test_token(&world.resources, &world.user).await;
    let routes = || OAuthGrantsRoutes::routes(Arc::clone(&world.resources));

    let listed: Value = AxumTestRequest::get("/api/me/oauth-standing-policies")
        .header("Authorization", &format!("Bearer {session}"))
        .send(routes())
        .await
        .assert_status(StatusCode::OK)
        .json();
    let listed = listed["policies"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "bug: the listing: {listed:?}");
    assert_eq!(
        (
            &listed[0]["id"],
            &listed[0]["client_id"],
            &listed[0]["preset"]
        ),
        (
            &Value::from(policy.id.clone()),
            &Value::from(client.id.clone()),
            &Value::from("training_coach")
        ),
        "bug: the listed policy"
    );
    assert_eq!(
        listed[0]["client_name"], "Grant details client",
        "bug: the client is not named"
    );

    // Another athlete can neither see nor remove it.
    let email = format!("policy_other_{}@example.test", Uuid::new_v4());
    let (_, other) = common::create_test_user_with_email(&world.resources.agent.database, &email)
        .await
        .unwrap();
    let other = common::generate_test_token(&world.resources, &other).await;
    let theirs: Value = AxumTestRequest::get("/api/me/oauth-standing-policies")
        .header("Authorization", &format!("Bearer {other}"))
        .send(routes())
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        theirs["policies"].as_array().map(Vec::len),
        Some(0),
        "bug: another athlete reads the policy"
    );
    let uri = format!("/api/me/oauth-standing-policies/{}", policy.id);
    AxumTestRequest::delete(&uri)
        .header("Authorization", &format!("Bearer {other}"))
        .send(routes())
        .await
        .assert_status(StatusCode::NOT_FOUND);
    AxumTestRequest::delete(&uri)
        .send(routes())
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    AxumTestRequest::delete(&uri)
        .header("Authorization", &format!("Bearer {session}"))
        .send(routes())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    AxumTestRequest::delete(&uri)
        .header("Authorization", &format!("Bearer {session}"))
        .send(routes())
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(policies(&world).await.is_empty(), "bug: the policy stands");
    assert_eq!(
        live_grant(&world, &client).await.map(|live| live.id),
        Some(grant.id.clone()),
        "bug: removing the policy touched its grant"
    );

    expire(&world, &grant.id).await;
    forget_standing(&world, &client).await;
    assert_eq!(
        authorize(&world, &client).await,
        200,
        "bug: a removed policy renewed"
    );
}

#[tokio::test]
async fn deleting_the_client_or_sweeping_its_orphan_revokes_its_policies() {
    let world = world().await;
    let client = register(&world).await;
    expired_under_policy(&world, &client).await;
    let repos = &world.resources.common.repos;
    repos.oauth2_server.delete_client(&client.id).await.unwrap();
    assert!(
        policies(&world).await.is_empty(),
        "bug: a deleted client's policy stands"
    );

    // A policy naming a client that is already gone goes with the sweep.
    let orphan = Client {
        id: format!("gone-{}", Uuid::new_v4().simple()),
        secret: String::new(),
    };
    repos
        .oauth_standing_policies
        .store_standing_policy(&OAuthStandingPolicy {
            id: Uuid::new_v4().to_string(),
            tenant_id: world.tenant_id.clone(),
            user_id: world.user.id.to_string(),
            client_id: orphan.id.clone(),
            preset: "sleep_coach".to_owned(),
            scope: "fitness:read".to_owned(),
            authorization_details: preset_terms(&world),
            consent_version: "digest".to_owned(),
            consent_locale: "en".to_owned(),
            created_at: Timestamp::now(),
            revoked_at: None,
        })
        .await
        .unwrap();
    assert_eq!(policies(&world).await.len(), 1, "fixture: an orphan policy");
    let long_ago = Timestamp::now() - Elapsed::days(10_000);
    repos
        .oauth2_server
        .delete_stale_clients(long_ago, long_ago, Timestamp::now())
        .await
        .unwrap();
    assert!(
        policies(&world).await.is_empty(),
        "bug: the sweep left an orphan's policy standing"
    );
}

#[tokio::test]
async fn the_box_ticked_beside_a_preset_stores_the_policy_that_renews_it() {
    let world = world().await;
    let client = register(&world).await;
    let screen =
        authorize_query_as(&world, &request_query(&client, &fresh_state(), &[]), true).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    for preset in Preset::ALL {
        assert!(
            page.contains(&format!(r#"name="standing_policy_{}""#, preset.name())),
            "bug: no box beside {}: {page}",
            preset.name()
        );
    }
    assert!(
        !page.contains(r#"name="standing_policy_everything""#),
        "bug: the plain choice carries a box: {page}"
    );
    let line = shown(
        &world,
        KEY_CONSENT_STANDING_POLICY,
        &[r#"<span class="client">Grant details client</span>"#],
    );
    assert_eq!(
        page.matches(line.as_str()).count(),
        Preset::ALL.len(),
        "bug: the box's line is not shown beside each preset: {page}"
    );

    let approved = approve_with(
        &world,
        &client,
        &[("standing_policy_training_coach", "on")],
        Some("training_coach"),
    )
    .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let grant = live_grant(&world, &client).await.expect("the preset grant");
    let stored = policies(&world).await;
    assert_eq!(
        stored.len(),
        1,
        "bug: {} policies from one box",
        stored.len()
    );
    let policy = &stored[0];
    assert_eq!(
        (
            policy.client_id.as_str(),
            policy.preset.as_str(),
            Some(policy.consent_version.as_str()),
            Some(policy.consent_locale.as_str()),
        ),
        (
            client.id.as_str(),
            "training_coach",
            grant.consent_version.as_deref(),
            grant.consent_locale.as_deref(),
        ),
        "bug: the policy is not the approved grant's preset, digest and language"
    );
    let details =
        ResolvedDetails::from_document(grant.authorization_details.as_deref().unwrap()).unwrap();
    let terms = ResolvedDetails::from_document(&policy.authorization_details).unwrap();
    assert!(
        details.same_terms(&terms),
        "bug: the policy does not hold the approved terms"
    );

    expire(&world, &grant.id).await;
    forget_standing(&world, &client).await;
    assert_eq!(
        authorize(&world, &client).await,
        303,
        "bug: the policy the box stored did not renew its grant"
    );
}

#[tokio::test]
async fn a_box_on_the_plain_choice_or_from_an_admin_creates_no_policy() {
    let world = world().await;
    let client = register(&world).await;
    let plain = approve_with(
        &world,
        &client,
        &[
            ("standing_policy_everything", "on"),
            ("standing_policy_training_coach", "on"),
        ],
        Some("everything"),
    )
    .await;
    assert_eq!(plain.status(), 303, "{}", plain.body_text());
    assert!(
        policies(&world).await.is_empty(),
        "bug: a box beside the plain choice created a policy"
    );

    let admin_client = register(&world).await;
    world
        .resources
        .common
        .repos
        .users
        .set_admin_status(world.user.id, true)
        .await
        .unwrap();
    let screen = authorize_query_as(
        &world,
        &request_query(&admin_client, &fresh_state(), &[]),
        true,
    )
    .await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    assert!(
        !screen.body_text().contains("standing_policy"),
        "bug: an admin's screen offers the box"
    );
    let forged = approve_with(
        &world,
        &admin_client,
        &[("standing_policy_training_coach", "on")],
        Some("training_coach"),
    )
    .await;
    assert_eq!(forged.status(), 303, "{}", forged.body_text());
    assert!(
        policies(&world).await.is_empty(),
        "bug: a forged box from an admin created a policy"
    );
}

#[tokio::test]
async fn storing_a_policy_again_replaces_the_live_one() {
    let world = world().await;
    let client = register(&world).await;
    let grant = approve_preset(&world, &client, "training_coach").await;
    let first = seed_policy(&world, &grant).await;
    let second = seed_policy(&world, &grant).await;
    let live = policies(&world).await;
    assert_eq!(
        live.iter()
            .map(|policy| policy.id.as_str())
            .collect::<Vec<_>>(),
        vec![second.id.as_str()],
        "bug: {} stands beside its replacement",
        first.id
    );
}
