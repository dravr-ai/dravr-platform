// ABOUTME: Integration tests for the hosted connect picker routes (page, oauth-init, success)
// ABOUTME: Pins connect-token gating (missing/invalid/narrow-scope), picker render, and the Strava-OAuth connected merge
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::{
    self as oauth_providers, TRAININGPEAKS_TERMS_VERSION,
};
use pierre_core::models::{ConnectionType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_link_token, MintProviderLinkTokenArgs,
};
use pierre_routes_auth::AuthRoutes;
use uuid::Uuid;

async fn test_setup() -> (Arc<ServerContext>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    (resources, user_id, tenant_id)
}

fn connect_token(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> String {
    mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token")
}

/// `compute_providers_status` reads connected state from `provider_connections`
/// (the single source of truth), so the merge test registers a real connection
/// row rather than seeding a bare oauth token.
async fn register_connection(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, provider, &ConnectionType::OAuth, None)
        .await
        .expect("register test connection");
}

// ============================================================================
// GET /providers/connect — token gating + picker render
// ============================================================================

#[tokio::test]
async fn connect_page_without_token_renders_error_page() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/providers/connect").send(app).await;

    assert_eq!(resp.status(), 200, "error page is a friendly 200 HTML page");
    let body = resp.text();
    assert!(
        body.contains("Missing connect token"),
        "missing-token copy expected: {body}"
    );
    assert!(
        !body.contains("\"provider\""),
        "no provider cards may render without a token"
    );
    assert!(
        body.contains("@media (prefers-color-scheme: dark)")
            && body.contains(
                r#"<div class="status-icon status-icon-error" aria-hidden="true">!</div>"#
            ),
        "the error page draws with the shared Boreal sheet, in both schemes"
    );
    assert!(!body.contains("{{"), "no placeholder survives the render");
}

#[tokio::test]
async fn connect_page_rejects_garbage_token() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/providers/connect?token=not-a-jwt")
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("invalid or has expired"),
        "invalid-token copy expected: {body}"
    );
    assert!(!body.contains("\"provider\""));
}

/// A narrow per-provider token (the Canot-minted `provider:sciotte:login`) must
/// never open the connect picker — only the connect superset scope may.
#[tokio::test]
async fn connect_page_rejects_narrow_sciotte_token() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let narrow = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&narrow)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("invalid or has expired"),
        "narrow scope must be rejected: {body}"
    );
    assert!(!body.contains("\"provider\""));
}

#[tokio::test]
async fn connect_page_renders_picker_for_valid_connect_token() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    // The cards are serialized as JSON into the page; the Strava-path and
    // Garmin-path cards must both be offered, and with no connection seeded
    // nothing may render as already connected.
    assert!(
        body.contains("\"target\":\"strava\""),
        "Strava-path card expected: {body}"
    );
    assert!(
        body.contains("\"target\":\"garmin\""),
        "Garmin-path card expected: {body}"
    );
    assert!(
        !body.contains("\"connected\":true"),
        "no card may show connected without a seeded connection"
    );
}

/// Regression: the raw `strava` OAuth row is hidden from the picker, but its
/// connected state must merge into the Strava-path card — an already-connected
/// user must not be offered a needless re-consent (web
/// `ProviderConnectionCards` parity).
#[tokio::test]
async fn connect_page_merges_strava_oauth_connection_into_card() {
    let (resources, user_id, tenant_id) = test_setup().await;
    register_connection(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("\"connected\":true"),
        "the Strava-path card must show connected when a strava OAuth row exists: {body}"
    );
}

/// Regression (carnet#352): a `garmin` OAuth row ALONE must not read as
/// connected. `resolve_backend` routes every Garmin request to `sciotte_garmin`
/// unconditionally, because the official Garmin API is partner-gated and
/// uncredentialed — so an athlete holding only the OAuth row saw
/// `connected: true, needs_reauth: false` on every status surface while each
/// coach call failed with "Provider `sciotte_garmin` requires authentication".
#[tokio::test]
async fn connect_page_does_not_show_garmin_connected_on_an_oauth_row_alone() {
    let (resources, user_id, tenant_id) = test_setup().await;
    register_connection(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        !body.contains("\"connected\":true"),
        "a garmin OAuth row serves no fetch, so no card may render connected: {body}"
    );
}

/// The other half of the pair, so the fix above is a correction and not simply
/// "Garmin is never connected": the mirror row is the one that actually serves,
/// and it must read as connected.
#[tokio::test]
async fn connect_page_shows_garmin_connected_on_the_mirror_row() {
    let (resources, user_id, tenant_id) = test_setup().await;
    register_connection(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("\"connected\":true"),
        "the sciotte_garmin row is the backend every Garmin fetch uses, so the card must show connected: {body}"
    );
}

// ============================================================================
// GET /api/providers/connect/oauth-init/{provider} — token gating
// ============================================================================

#[tokio::test]
async fn oauth_init_rejects_missing_token() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/api/providers/connect/oauth-init/strava")
        .send(app)
        .await;

    assert_ne!(resp.status(), 302, "no redirect without a token");
    assert!(
        resp.status() >= 400,
        "missing token must be an auth error, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn oauth_init_rejects_narrow_sciotte_token() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let narrow = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    let resp = AxumTestRequest::get(&format!(
        "/api/providers/connect/oauth-init/strava?token={}",
        urlencoding::encode(&narrow)
    ))
    .send(app)
    .await;

    assert_ne!(resp.status(), 302, "narrow scope must not initiate OAuth");
    assert!(
        resp.status() >= 400,
        "narrow token must be an auth error, got {}",
        resp.status()
    );
}

/// An unknown provider path param must never 302 — the provider registry is
/// the allowlist, and the failure renders the friendly error page.
#[tokio::test]
async fn oauth_init_rejects_unknown_provider() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/api/providers/connect/oauth-init/not_a_provider?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_ne!(resp.status(), 302, "unknown provider must not redirect");
    let body = resp.text();
    assert!(
        body.contains("We couldn&#x27;t start the connection for this provider."),
        "unknown provider renders the error path: {body}"
    );
}

// ============================================================================
// GET /providers/connect/success
// ============================================================================

#[tokio::test]
async fn success_page_renders_with_channel_and_target() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/providers/connect/success?channel=telegram&target=strava")
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.to_lowercase().contains("telegram"),
        "success page should reference the originating channel: {body}"
    );
}

/// TrainingPeaks has only the scrape path, so the picker offers it as a
/// credential card, and its mirror row is what lights it.
#[tokio::test]
async fn connect_page_offers_trainingpeaks_and_lights_it_on_the_mirror_row() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let url = format!(
        "/providers/connect?token={}",
        urlencoding::encode(&connect_token(&resources, user_id, tenant_id))
    );

    let before = AxumTestRequest::get(&url)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(before.status(), 200);
    let body = before.text();
    assert!(
        body.contains("\"provider\":\"sciotte_trainingpeaks\"")
            && body.contains("\"target\":\"trainingpeaks\""),
        "a TrainingPeaks credential card is offered: {body}"
    );
    assert!(!body.contains("\"connected\":true"));

    register_connection(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    let after = AxumTestRequest::get(&url)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
        .text();
    assert!(
        after.contains("\"connected\":true"),
        "the sciotte_trainingpeaks row must show the TrainingPeaks card connected: {after}"
    );
}

#[tokio::test]
async fn success_page_names_trainingpeaks() {
    let (resources, _, _) = test_setup().await;
    let resp =
        AxumTestRequest::get("/providers/connect/success?channel=telegram&target=trainingpeaks")
            .send(AuthRoutes::routes(resources.auth_routes_context()))
            .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("TrainingPeaks"),
        "the success page names the provider just connected, not a generic label: {body}"
    );
}

// ============================================================================
// The TrainingPeaks exposure notice on the hosted pages
// ============================================================================

/// The hosted login page for `target`, rendered from a freshly minted link
/// token (each render burns its token).
async fn hosted_login_page(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    target: &str,
) -> String {
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target,
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint hosted-login token");
    let resp = AxumTestRequest::get(&format!(
        "/providers/sciotte/login?token={}",
        urlencoding::encode(&token)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await;
    assert_eq!(resp.status(), 200);
    resp.text()
}

/// The hosted TrainingPeaks login states the exposure above the credentials
/// and holds Log In until the box is ticked — until this account has accepted
/// the notice, after which a reconnect goes straight to the form. Garmin never
/// shows it.
#[tokio::test]
async fn hosted_login_shows_the_trainingpeaks_notice_until_the_account_accepts_it() {
    let (resources, user_id, tenant_id) = test_setup().await;

    let page = hosted_login_page(&resources, user_id, tenant_id, "trainingpeaks").await;
    assert!(
        page.contains("TrainingPeaks could suspend my account"),
        "the acceptance is stated on the page: {page}"
    );
    assert!(page.contains("Terms of Use (section 13)"));
    assert!(
        page.contains(
            // The page HTML-escapes the apostrophe.
            "If yours is a human coach&#x27;s account, Dravr also uses it to read the \
             calendars of the athletes who confirm a link in a group you oversee."
        ),
        "the notice says a coach's account reads the athletes who confirm a link: {page}"
    );
    assert!(
        page.contains("id=\"consent-block\">"),
        "the notice starts visible"
    );
    // The notice opens with its title, as the web and mobile modals show it,
    // on the warning callout with the acceptance as its outlined checkbox.
    assert!(
        page.contains(r#"<div class="callout" role="note" id="consent-block">"#)
            && page.contains(r#"<p class="callout-title">Before you connect TrainingPeaks</p>"#),
        "the hosted notice carries its title: {page}"
    );
    assert!(
        page.find("Before you connect TrainingPeaks").unwrap()
            < page.find("Terms of Use (section 13)").unwrap(),
        "the title heads the notice body"
    );
    assert!(page.contains(r#"<label class="check"><input type="checkbox" id="tos-consent">"#));
    assert!(
        page.contains("@media (prefers-color-scheme: dark)"),
        "the login page draws with the shared Boreal sheet, in both schemes"
    );
    assert!(!page.contains("{{"), "no placeholder survives the render");
    assert!(page.contains("consentRequired: true"));
    // TrainingPeaks signs in with a username; an email-typed field would
    // refuse the account before it was ever sent.
    assert!(
        page.contains("<label for=\"email\">Username</label>")
            && page.contains("type=\"text\" id=\"email\"")
            && page.contains("autocomplete=\"username\""),
        "the TrainingPeaks login asks for a username"
    );
    let notice_at = page.find("id=\"consent-block\"").unwrap();
    let form_at = page.find("id=\"login-form\"").unwrap();
    assert!(
        notice_at < form_at,
        "the notice comes before the credentials, not after them"
    );

    resources
        .common
        .repos
        .users
        .record_trainingpeaks_terms(user_id, TRAININGPEAKS_TERMS_VERSION)
        .await
        .unwrap();
    let accepted = hosted_login_page(&resources, user_id, tenant_id, "trainingpeaks").await;
    assert!(accepted.contains("consentRequired: false"));
    assert!(accepted.contains("id=\"consent-block\" hidden>"));

    let garmin = hosted_login_page(&resources, user_id, tenant_id, "garmin").await;
    assert!(garmin.contains("consentRequired: false"));
    assert!(
        garmin.contains("<label for=\"email\">Email</label>")
            && garmin.contains("type=\"email\" id=\"email\""),
        "Garmin keeps its email login"
    );
    assert!(garmin.contains("id=\"consent-block\" hidden>"));
}

/// The picker carries the notice hidden, and tells the page per card whether
/// to show it — only the TrainingPeaks card asks, and only until accepted.
#[tokio::test]
async fn picker_asks_for_the_trainingpeaks_notice_on_that_card_alone() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let url = format!(
        "/providers/connect?token={}",
        urlencoding::encode(&connect_token(&resources, user_id, tenant_id))
    );

    let body = AxumTestRequest::get(&url)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
        .text();
    assert!(body.contains("id=\"consent-block\" hidden>"));
    assert!(body.contains("TrainingPeaks could suspend my account"));
    assert!(
        body.contains(r#"<p class="callout-title">Before you connect TrainingPeaks</p>"#),
        "the picker's notice carries its title too"
    );
    // A card's glyph takes its provider's ink per scheme, from PROVIDER_GLYPH_INK.
    assert!(body.contains(r#".pc-glyph[data-provider="sciotte_trainingpeaks"]"#));
    assert!(
        body.contains("\"target\":\"trainingpeaks\",\"consent_required\":true"),
        "the TrainingPeaks card asks for the notice: {body}"
    );
    assert!(
        body.contains("\"target\":\"garmin\",\"consent_required\":false"),
        "the Garmin card does not: {body}"
    );
    assert!(
        body.contains("\"consent_required\":true,\"login_identifier\":\"username\"")
            && body.contains(
                "\"target\":\"garmin\",\"consent_required\":false,\"login_identifier\":\"email\""
            ),
        "each card tells the page what its provider signs in with: {body}"
    );
    assert!(
        body.contains("'Username / password'"),
        "the picker names the username a TrainingPeaks card asks for"
    );

    resources
        .common
        .repos
        .users
        .record_trainingpeaks_terms(user_id, TRAININGPEAKS_TERMS_VERSION)
        .await
        .unwrap();
    let accepted = AxumTestRequest::get(&url)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
        .text();
    assert!(
        accepted.contains("\"target\":\"trainingpeaks\",\"consent_required\":false"),
        "an account that accepted is not asked again: {accepted}"
    );
}

// ============================================================================
// GET /r/{code} — the expired short-link page
// ============================================================================

/// A lapsed short link lands on a hosted page like the others: the shared
/// Boreal sheet in both schemes, the lockup, and the copy in the browser's
/// language.
#[tokio::test]
async fn expired_short_link_page_draws_with_the_boreal_sheet() {
    let (resources, _, _) = test_setup().await;
    let resp = AxumTestRequest::get("/r/0123456789abcdef0123456789abcdef")
        .header("accept-language", "en-CA,en;q=0.9")
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;

    assert_eq!(resp.status(), 404);
    let body = resp.text();
    assert!(
        body.contains("<h1>This link has expired</h1>"),
        "the page speaks the browser's language: {body}"
    );
    assert!(body.contains(r#"<div class="lockup" role="img" aria-label="Dravr"></div>"#));
    let dark_at = body
        .find("@media (prefers-color-scheme: dark)")
        .expect("the page carries the dark scheme");
    assert!(body[..dark_at].contains("--color-primary: 37 95 77;"));
    assert!(body[dark_at..].contains("--color-primary: 163 208 190;"));
    assert!(
        !body.contains("style=\""),
        "no inline style is left on the page"
    );
}
