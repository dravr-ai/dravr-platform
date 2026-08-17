// ABOUTME: Asserts provider.disconnected fires from the domain chokepoint on every disconnect surface
// ABOUTME: Plus the /mcp carve-out removing the provider_connections row it used to leave orphaned

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for dravr-carnet#29.
//!
//! `provider.disconnected` was emitted only from the REST route, so a
//! disconnect through the chat tool loop or the `/mcp` + SSE carve-out was
//! invisible to `PostHog` — connects and disconnects were counted on different
//! surfaces. Worse, the carve-out hand-rolled a raw-name token delete: it
//! never resolved the sciotte mirror backend and never removed the
//! `provider_connections` row, so a chat user who disconnected kept a row
//! claiming the provider was still connected.
//!
//! Every disconnect surface now funnels through
//! `OAuthService::disconnect_provider`. These tests drive each surface and
//! assert on the emitted event's fields AND on both storage rows, so a
//! regression that re-splits the paths (or re-orphans the connection row)
//! fails loudly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use common::{create_test_server_resources, create_test_user};
use pierre_auth::tenant::TenantContext;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::tool_handlers::ToolRoutingContext;
use pierre_services::oauth_flow::OAuthService;
use pierre_tool_runtime::implementations::connection::DisconnectProviderTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

use dravr_tronc::mcp::tool::{McpTool, ToolContext};

// ============================================================================
// Notify-event capture
// ============================================================================

/// One `target: "notify"` event, with every field rendered as a string.
#[derive(Clone, Debug)]
struct NotifyEvent {
    event: String,
    fields: HashMap<String, String>,
}

impl NotifyEvent {
    fn field(&self, name: &str) -> &str {
        self.fields
            .get(name)
            .unwrap_or_else(|| panic!("event {} has no field {name}", self.event))
    }
}

#[derive(Clone, Default)]
struct NotifyCapture {
    events: Arc<Mutex<Vec<NotifyEvent>>>,
}

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S> Layer<S> for NotifyCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "notify" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event")
            .cloned()
            .unwrap_or_else(|| panic!("notify event with no `event` field: {visitor:?}"));
        self.events.lock().unwrap().push(NotifyEvent {
            event: name,
            fields: visitor.fields,
        });
    }
}

/// Install a capture subscriber for the current thread.
///
/// The guard must stay alive for the duration of the code under test — the
/// subscriber is uninstalled when it drops.
fn capture_notify() -> (Arc<Mutex<Vec<NotifyEvent>>>, DefaultGuard) {
    let capture = NotifyCapture::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

/// Exactly one event with this name, or a panic naming what was seen instead.
fn only(events: &Arc<Mutex<Vec<NotifyEvent>>>, name: &str) -> NotifyEvent {
    let all = events.lock().unwrap();
    let matching: Vec<NotifyEvent> = all.iter().filter(|e| e.event == name).cloned().collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one `{name}`, saw {:?}",
        all.iter().map(|e| e.event.clone()).collect::<Vec<_>>()
    );
    matching.into_iter().next().unwrap()
}

// ============================================================================
// Fixtures
// ============================================================================

async fn user_primary_tenant(resources: &Arc<ServerContext>, user_id: Uuid) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id
}

/// Seed a token row + a connection row for `backend`, exactly as the connect
/// paths write them.
async fn seed_connected_provider(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    connection_type: &ConnectionType,
) {
    let token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        backend.to_owned(),
        "test_access_token".to_owned(),
        Some("test_refresh_token".to_owned()),
        Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("upsert test token");
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, backend, connection_type, None)
        .await
        .expect("register test connection");
}

/// Assert both sources of truth are gone for `backend`.
async fn assert_fully_disconnected(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let token = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
        .unwrap();
    assert!(token.is_none(), "the {backend} token row must be deleted");

    let conns = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        !conns.iter().any(|c| c.provider == backend),
        "the {backend} connection row must be removed, not left orphaned"
    );
}

fn assert_event_attributed(
    event: &NotifyEvent,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    assert_eq!(event.field("provider"), provider);
    assert_eq!(event.field("user_id"), user_id.to_string());
    assert_eq!(event.field("tenant_id"), tenant_id.to_string());
}

// ============================================================================
// Service chokepoint (REST path)
// ============================================================================

/// The service deletes the token + connection row in lockstep and emits a
/// fully-attributed `provider.disconnected`. A stale copy that skipped the
/// connection row, or an emit that relied on span fields, fails here.
#[tokio::test]
async fn service_disconnect_cleans_both_rows_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::STRAVA,
        &ConnectionType::OAuth,
    )
    .await;

    let (events, _guard) = capture_notify();
    let service = OAuthService::new(resources.data(), resources.common.config.clone(), None);
    service
        .disconnect_provider(user_id, oauth_providers::STRAVA, Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect must succeed");

    assert_fully_disconnected(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let event = only(&events, "provider.disconnected");
    assert_event_attributed(&event, user_id, tenant_id, oauth_providers::STRAVA);
}

// ============================================================================
// Chat tool loop (registry DisconnectProviderTool)
// ============================================================================

/// The chat tool resolves the sciotte mirror ("garmin" → `sciotte_garmin`),
/// cleans both rows for the mirror backend, and emits the event under the
/// user-facing provider name — the same name `provider.connected` uses, so
/// the connect/disconnect pair is measured on one axis.
#[tokio::test]
async fn chat_tool_disconnect_resolves_mirror_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
        &ConnectionType::Manual,
    )
    .await;

    let (events, _guard) = capture_notify();
    let tool = DisconnectProviderTool;
    let state: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer");
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;
    assert!(
        !result.is_error,
        "disconnect must succeed: {:?}",
        result.structured_content
    );

    assert_fully_disconnected(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    let event = only(&events, "provider.disconnected");
    assert_event_attributed(&event, user_id, tenant_id, oauth_providers::GARMIN);
}

// ============================================================================
// The /mcp + SSE carve-out (dravr-carnet#29's stale-row path)
// ============================================================================

/// The carve-out used to delete only the raw-named token — no connection-row
/// removal, no event. Driving `route_disconnect_tool` must now leave no
/// orphaned row and must emit the attributed event, exactly like the other
/// surfaces.
#[tokio::test]
async fn mcp_carveout_disconnect_removes_connection_row_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::STRAVA,
        &ConnectionType::OAuth,
    )
    .await;

    let (events, _guard) = capture_notify();
    let tenant_context =
        TenantContext::for_tenant_scoped_operation(tenant_id, "Test Tenant".to_owned(), user_id);
    let tool_context = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer");
    let routing_ctx = ToolRoutingContext {
        resources: &resources,
        tenant_context: &tenant_context,
        tool_context: &tool_context,
    };
    let response =
        ProviderToolRouter::route_disconnect_tool(oauth_providers::STRAVA, json!(1), &routing_ctx)
            .await;
    assert!(
        response.error.is_none(),
        "disconnect must succeed: {:?}",
        response.error
    );

    assert_fully_disconnected(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let event = only(&events, "provider.disconnected");
    assert_event_attributed(&event, user_id, tenant_id, oauth_providers::STRAVA);
}
