// ABOUTME: The hosted import page and its import link-token: start, follow and cancel only, never the report or the delete
// ABOUTME: Foreign, expired and out-of-tenant tokens are refused; the connect picker offers the page beside its provider cards

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The hosted import page (carnet#1221).
//!
//! A chat's connect picker opens `/providers/connect/import` with its connect
//! link-token; the page narrows it into an import link-token and calls
//! `/providers/connect/import/{source}` with it. These tests drive those
//! routes on barrage's memory stores and the synthetic export the archive
//! import tests serve, as the page's script does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::http::StatusCode;
use chrono::Utc;
use pierre_core::constants::oauth_providers::STRAVA_ARCHIVE;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::archive_import::ArchiveImports;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_import_link_token, mint_link_token, verify_link_token,
    MintProviderLinkTokenArgs, CONNECT_PROVIDER,
};
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use tokio::time;
use uuid::Uuid;

use crate::archive_import_test::{
    call, imports_serving, seed_athlete, strava_export, Athlete, LINK, REFUSED_LINK,
};
use crate::common;
use crate::helpers::axum_test::AxumTestRequest;

/// Where the page's script starts an import of the Strava export.
const START: &str = "/providers/connect/import/strava_archive";

/// Where the page's script follows it.
const CURRENT: &str = "/providers/connect/import/strava_archive/current";

/// A server context whose imports serve the synthetic Strava export.
async fn importing_server() -> Arc<ServerContext> {
    let (imports, _) = imports_serving(strava_export());
    common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context")
}

/// The connect link-token a chat would send `athlete`.
fn connect_token(resources: &ServerContext, user_id: Uuid, tenant: TenantId) -> String {
    mint_connect_link_token(
        user_id,
        tenant.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("a connect token")
}

/// The import link-token the hosted page mints from `connect`, with `exp`
/// overriding the connect token's expiry when given.
fn import_token_from(resources: &ServerContext, connect: &str, exp: Option<i64>) -> String {
    let mut claims = verify_link_token(connect, &resources.auth.admin_jwt_secret, CONNECT_PROVIDER)
        .expect("the connect token verifies");
    if let Some(exp) = exp {
        claims.exp = exp;
    }
    mint_import_link_token(&claims, STRAVA_ARCHIVE, &resources.auth.admin_jwt_secret)
        .expect("an import token")
}

/// The import token `athlete`'s hosted page holds.
fn import_token(resources: &ServerContext, athlete: &Athlete) -> String {
    let connect = connect_token(resources, athlete.user_id, athlete.tenant);
    import_token_from(resources, &connect, None)
}

/// Follow the import with `token` until it is final.
async fn follow_until_final(resources: &Arc<ServerContext>, token: &str) -> Value {
    for _ in 0..200 {
        let (status, body) = call(resources, "GET", CURRENT, token, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let state = body["import"]["status"].as_str().unwrap_or_default();
        if ["completed", "failed", "cancelled"].contains(&state) {
            return body["import"].clone();
        }
        time::sleep(StdDuration::from_millis(25)).await;
    }
    panic!("the import did not finish");
}

#[tokio::test]
async fn an_import_link_token_starts_and_follows_an_import() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-import").await;
    let token = import_token(&resources, &athlete);

    let (status, started) = call(
        &resources,
        "POST",
        START,
        &token,
        Some(json!({ "url": LINK })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    assert!(
        started.get("url").is_none(),
        "the link is never served back"
    );

    let finished = follow_until_final(&resources, &token).await;
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["imported"], 2, "{finished}");
    assert_eq!(finished["history_held"], true, "{finished}");

    // The import is the athlete's own: their session in the app sees it.
    let (_, current) = call(
        &resources,
        "GET",
        "/api/imports/strava_archive/current",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(current["import"]["id"], finished["id"], "{current}");
}

#[tokio::test]
async fn an_import_link_token_waits_for_the_link_and_cancels() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-await").await;
    let token = import_token(&resources, &athlete);

    let (status, waiting) = call(&resources, "POST", START, &token, None).await;
    assert_eq!(status, StatusCode::CREATED, "{waiting}");
    assert_eq!(waiting["status"], "awaiting_link", "{waiting}");
    let id = waiting["id"].as_str().expect("an id").to_owned();

    let (status, cancelled) = call(
        &resources,
        "POST",
        &format!("{START}/{id}/cancel"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["cancelled"], true, "{cancelled}");
    let (_, current) = call(&resources, "GET", CURRENT, &token, None).await;
    assert_eq!(current["import"]["status"], "cancelled", "{current}");
}

#[tokio::test]
async fn an_import_link_token_reads_no_report_and_deletes_nothing() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-scope").await;
    let token = import_token(&resources, &athlete);
    let (status, started) = call(
        &resources,
        "POST",
        START,
        &token,
        Some(json!({ "url": LINK })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    let finished = follow_until_final(&resources, &token).await;
    let id = finished["id"].as_str().expect("an id");

    let (report, body) = call(
        &resources,
        "GET",
        &format!("/api/imports/strava_archive/{id}/report"),
        &token,
        None,
    )
    .await;
    assert!(
        matches!(report, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "the import token reads no report: {report} {body}"
    );

    let (delete, body) = call(
        &resources,
        "DELETE",
        "/api/imports/strava_archive/data",
        &token,
        None,
    )
    .await;
    assert!(
        matches!(delete, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "the import token deletes nothing: {delete} {body}"
    );
    let held = resources
        .common
        .repos
        .activity_cache
        .held_user_provided(athlete.user_id, &athlete.tenant)
        .await
        .expect("the cache reads");
    assert_eq!(held, [STRAVA_ARCHIVE], "the imported history is still held");

    let (list, body) = call(&resources, "GET", "/api/imports", &token, None).await;
    assert!(
        matches!(list, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "no /api/imports route takes the import token: {list} {body}"
    );
    let (report, body) = call(
        &resources,
        "GET",
        &format!("/api/imports/strava_archive/{id}/report"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(report, StatusCode::OK, "the session still reads it: {body}");
}

#[tokio::test]
async fn connect_provider_expired_and_other_source_tokens_are_refused() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-foreign").await;
    let secret = &resources.auth.admin_jwt_secret;
    let connect = connect_token(&resources, athlete.user_id, athlete.tenant);
    let provider = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id: athlete.user_id,
            tenant_id: athlete.tenant.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        secret,
    )
    .expect("a provider token");
    let expired = import_token_from(&resources, &connect, Some(Utc::now().timestamp() - 3600));
    let claims = verify_link_token(&connect, secret, CONNECT_PROVIDER).expect("verify");
    let other_source = mint_import_link_token(&claims, "garmin_archive", secret).expect("mint");

    for (what, token) in [
        ("connect", &connect),
        ("provider", &provider),
        ("expired", &expired),
        ("other source", &other_source),
        ("session", &athlete.token),
    ] {
        let (status, body) = call(
            &resources,
            "POST",
            START,
            token,
            Some(json!({ "url": LINK })),
        )
        .await;
        assert!(
            matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "a {what} token starts no hosted import: {status} {body}"
        );
        let (status, body) = call(&resources, "GET", CURRENT, token, None).await;
        assert!(
            matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "a {what} token follows no hosted import: {status} {body}"
        );
    }
    let (_, current) = call(
        &resources,
        "GET",
        "/api/imports/strava_archive/current",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(current["import"], Value::Null, "nothing started: {current}");

    let token = import_token(&resources, &athlete);
    let (status, _) = call(
        &resources,
        "POST",
        "/providers/connect/import/garmin_archive",
        &token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "only offered sources are importable"
    );
}

#[tokio::test]
async fn an_import_link_token_reaches_only_its_own_athletes_import() {
    let resources = importing_server().await;
    let owner = seed_athlete(&resources, "hosted-owner").await;
    let other = seed_athlete(&resources, "hosted-other").await;
    let owner_token = import_token(&resources, &owner);
    let other_token = import_token(&resources, &other);
    let (_, waiting) = call(&resources, "POST", START, &owner_token, None).await;
    let id = waiting["id"].as_str().expect("an id").to_owned();

    let (status, body) = call(
        &resources,
        "POST",
        &format!("{START}/{id}/cancel"),
        &other_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another athlete's import is not found: {body}"
    );
    let (_, theirs) = call(&resources, "GET", CURRENT, &other_token, None).await;
    assert_eq!(theirs["import"], Value::Null, "{theirs}");
    let (_, mine) = call(&resources, "GET", CURRENT, &owner_token, None).await;
    assert_eq!(mine["import"]["status"], "awaiting_link", "{mine}");

    // A link naming the athlete in a tenant they do not belong to acts there
    // on nothing.
    let foreign = connect_token(
        &resources,
        owner.user_id,
        TenantId::from_uuid(Uuid::new_v4()),
    );
    let foreign = import_token_from(&resources, &foreign, None);
    let (status, body) = call(&resources, "POST", START, &foreign, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn a_refused_link_answers_with_its_refusal_code() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-refused").await;
    let token = import_token(&resources, &athlete);

    let (status, body) = call(
        &resources,
        "POST",
        START,
        &token,
        Some(json!({ "url": REFUSED_LINK })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("link_host_not_allowed:")),
        "the page reads the code the message leads with: {body}"
    );
}

/// A reason as `ArchiveImports::from_env` words a malformed fetch allowlist.
const DISABLED_REASON: &str = "export fetch client: ARCHIVE_FETCH_HOSTS: allowlist entry \
                               `s3.amazonaws.com/a/../b/` has an invalid path prefix";

#[tokio::test]
async fn a_server_without_imports_says_so_to_the_hosted_page() {
    let resources = common::create_test_server_resources_with_archive_imports(
        ArchiveImports::disabled(DISABLED_REASON),
    )
    .await
    .expect("a server context");
    let athlete = seed_athlete(&resources, "hosted-disabled").await;
    let token = import_token(&resources, &athlete);

    let (status, body) = call(&resources, "GET", CURRENT, &token, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], false, "{body}");

    // Starting one is refused, and the operator's configuration that disabled
    // imports stays in the boot log: the athlete reads no part of it.
    let (status, body) = call(&resources, "POST", START, &token, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        !body.to_string().contains("ARCHIVE_FETCH_HOSTS"),
        "the disabled reason reached the athlete: {body}"
    );

    // The picker offers no import, and the page says imports do not run here.
    let connect = connect_token(&resources, athlete.user_id, athlete.tenant);
    let picker = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&connect)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await
    .text();
    assert!(!picker.contains("id=\"import-own-data\""), "{picker}");
    let page = AxumTestRequest::get(&format!(
        "/providers/connect/import?token={}",
        urlencoding::encode(&connect)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await
    .text();
    assert!(page.contains("enabled: false"), "{page}");
}

#[tokio::test]
async fn the_picker_offers_the_import_beside_the_credential_connect() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-picker").await;
    resources
        .common
        .repos
        .users
        .update_locale(athlete.user_id, "en")
        .await
        .expect("store the athlete's locale");
    let connect = connect_token(&resources, athlete.user_id, athlete.tenant);

    let picker = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&connect)
    ))
    .header("accept-language", "en")
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await;
    assert_eq!(
        picker.status(),
        200,
        "the picker renders for a valid connect token"
    );
    let picker = picker.text();
    let import_href = format!(
        "href=\"/providers/connect/import?token={}\"",
        urlencoding::encode(&connect)
    );
    assert!(
        picker.contains(&import_href),
        "the picker links the import page with its own token: {picker}"
    );
    assert!(picker.contains(">Import your own data</a>"), "{picker}");
    // Scraping is never retired: the Strava and Garmin cards stay, and the
    // credential sign-in stays offered beside the import.
    assert!(picker.contains("\"target\":\"strava\""), "{picker}");
    assert!(picker.contains("\"target\":\"garmin\""), "{picker}");
    assert!(picker.contains("\"kind\":\"credentials\""), "{picker}");
    assert!(picker.contains("id=\"phase-credentials\""), "{picker}");
}

#[tokio::test]
async fn the_import_page_carries_an_import_token_and_never_the_connect_token_to_its_script() {
    let resources = importing_server().await;
    let athlete = seed_athlete(&resources, "hosted-page").await;
    let connect = connect_token(&resources, athlete.user_id, athlete.tenant);

    let page = AxumTestRequest::get(&format!(
        "/providers/connect/import?token={}",
        urlencoding::encode(&connect)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await;
    assert_eq!(
        page.status(),
        200,
        "the import page renders for a valid connect token"
    );
    let page = page.text();
    assert!(
        page.contains("<meta name=\"referrer\" content=\"no-referrer\">"),
        "the page sends no referrer carrying its link: {page}"
    );
    assert!(page.contains("enabled: true"), "{page}");

    let script_token = page
        .split("token: \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the script carries a token");
    assert_ne!(
        script_token, connect,
        "the script never holds the connect token"
    );
    let (status, body) = call(&resources, "GET", CURRENT, script_token, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the page's token follows the import: {body}"
    );
    // The connect token appears once: in the Back link to the picker.
    assert_eq!(
        page.matches(urlencoding::encode(&connect).as_ref()).count(),
        1,
        "{page}"
    );

    for refused in ["", "not-a-token"] {
        let page = AxumTestRequest::get(&format!("/providers/connect/import?token={refused}"))
            .header("accept-language", "en")
            .send(AuthRoutes::routes(resources.auth_routes_context()))
            .await
            .text();
        assert!(
            !page.contains("token: \""),
            "no token for a refused link: {page}"
        );
    }
}
