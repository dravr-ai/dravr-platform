// ABOUTME: The two tool-quota refusals public_mcp_monitoring.tf alerts on are the two the server logs, word for word
// ABOUTME: Drives a real /mcp dispatch over a cap and over an unreadable counter table, and pins the fail-closed answer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A log-based alert reads the words of a log line. Reworded on one side only,
//! the alert never fires and nothing says so, so each test here reads the
//! filter out of the Terraform and makes the server log the line.

use crate::common;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::models::{TenantId, STARTER};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::tool_handlers::ToolHandlers;
use pierre_runtime_context::default_admin_config;
use pierre_services::usage_counter::UsageCounterService;
use pierre_tool_runtime::runtime::ToolRuntime;
use regex::Regex;
use serde_json::{json, Value};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

/// A read-only tool that needs no provider connection, so only the quota ladder
/// can refuse the call.
const HARMLESS_TOOL: &str = "get_connection_status";

/// Usage this many times the starter cap is past it whatever burst the
/// counter service allows on top.
const OVER_THE_CAP: i64 = 10;

/// Breaks the read the quota ladder makes, on either backend.
const DROP_COUNTERS: &str = "DROP TABLE usage_counters";

#[derive(Default)]
struct MessageCapture {
    messages: Arc<Mutex<Vec<String>>>,
}

struct MessageVisitor<'a>(&'a mut String);

impl Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for MessageCapture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        self.messages.lock().unwrap().push(message);
    }
}

/// What the metric named `resource` in `public_mcp_monitoring.tf` matches a
/// message's start against.
fn alerted_prefix(resource: &str) -> String {
    let terraform = include_str!("../../../../infra/environments/dev/public_mcp_monitoring.tf");
    let filter = Regex::new(&format!(
        r#"(?s)resource "google_logging_metric" "{resource}" \{{.*?jsonPayload\.message=~"\^([^"]+)""#
    ))
    .unwrap();
    filter
        .captures(terraform)
        .unwrap_or_else(|| panic!("public_mcp_monitoring.tf has no message filter on {resource}"))
        [1]
    .to_owned()
}

async fn athlete() -> (Arc<ServerContext>, Uuid, TenantId) {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("tool_quota_{}@example.com", Uuid::new_v4());
    let (user_id, _user) = common::create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    let tenants = resources.common.repos.tenants.get_all().await.unwrap();
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("a fresh user owns a tenant");
    (Arc::clone(&resources), user_id, tenant.id)
}

/// A real `/mcp` tool dispatch: its JSON-RPC response and the log lines it wrote.
async fn dispatch(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> (Value, Vec<String>) {
    let state: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    let tool_context = ToolContext {
        user_id: Some(user_id.to_string()),
        tenant_id: Some(tenant_id.to_string()),
        auth_method: Some("jwt_bearer".to_owned()),
        request_id: Some(json!(1)),
        is_admin: false,
        scopes: OAuthScope::self_grant()
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect(),
        ..Default::default()
    };

    let capture = MessageCapture::default();
    let messages = Arc::clone(&capture.messages);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    let response = ToolHandlers::dispatch_tool_call(
        resources,
        &state,
        &tool_context,
        user_id,
        tenant_id,
        HARMLESS_TOOL,
        json!({}),
    )
    .await;
    drop(guard);

    let logged = messages.lock().unwrap().clone();
    (serde_json::to_value(&response).unwrap(), logged)
}

#[tokio::test]
async fn the_quota_refusal_the_alert_counts_is_the_one_the_server_logs() {
    let wanted = alerted_prefix("mcp_tool_quota_exceeded");
    let (resources, user_id, tenant_id) = athlete().await;
    let past_the_cap = STARTER.daily_messages * OVER_THE_CAP;
    UsageCounterService::new(
        resources.common.repos.usage_counters.as_ref(),
        default_admin_config(),
    )
    .increment(
        &tenant_id.to_string(),
        &user_id.to_string(),
        "daily_messages",
        past_the_cap,
    )
    .await
    .unwrap();

    let (response, logged) = dispatch(&resources, user_id, tenant_id).await;

    assert!(
        response.to_string().contains("Rate limit exceeded"),
        "{response}"
    );
    assert!(
        logged.iter().any(|message| message.starts_with(&wanted)),
        "the metric counts {wanted:?}; the server logged {logged:#?}"
    );
}

/// The quota is read from `usage_counters`; with that table gone the read
/// fails, and the call is refused rather than run unmetered.
#[tokio::test]
async fn the_fail_closed_refusal_the_alert_pages_on_is_the_one_the_server_logs() {
    let wanted = alerted_prefix("mcp_tool_quota_check_failed");
    let (resources, user_id, tenant_id) = athlete().await;
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(DROP_COUNTERS).execute(db.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(DROP_COUNTERS).execute(db.pool()).await.unwrap();
        }
    }

    let (response, logged) = dispatch(&resources, user_id, tenant_id).await;

    assert!(
        response.to_string().contains("quota_unavailable"),
        "{response}"
    );
    assert!(
        logged.iter().any(|message| message.starts_with(&wanted)),
        "the alert pages on {wanted:?}; the server logged {logged:#?}"
    );
}
