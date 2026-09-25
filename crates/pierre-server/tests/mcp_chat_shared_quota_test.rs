// ABOUTME: One quota policy, two doors — /mcp and a chat turn refuse the same user at the same number
// ABOUTME: Driven through both real entry points at the flip value; an /mcp refusal reaches the client with its data

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#95: `POST /mcp` mirrored the chat quota policy instead of sharing it.
//!
//! `McpToolHandlers::check_tool_quota` resolved the tier itself, built its own
//! `UsageCounterService`, ran its own ladder, and exempted the admin role —
//! which the chat policy explicitly does not. Its own comment said it mirrored
//! the chat route, and a mirror is a copy that drifts: the same account
//! refused at two different points depending on which door it knocked on, and
//! registre#9 is what that costs when nobody notices for months.
//!
//! The threshold is asserted by value at the boundary rather than "both
//! refuse eventually": one counter value below the hard limit both surfaces
//! allow, and at the hard limit both refuse. A test that only checked the
//! refusal would pass against two ladders that happen to be far apart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "client-chat")]
mod shared_quota_tests {
    use crate::common;
    use anyhow::Result;
    use axum::body::{to_bytes, Body};
    use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
    use axum::http::{Request, StatusCode};
    use chrono::{Duration, Utc};
    use dravr_tronc::mcp::tool::ToolContext;
    use pierre_chat_pipeline::quota_policy::{check_pre_chat_quotas_scoped, PreChatScope};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{TenantId, STARTER};
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_database::backends::factory::Database;
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::mcp::tool_handlers::ToolHandlers;
    use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
    use pierre_services::quota_policy::{check_quotas, QuotaPolicyInputs, QuotaSurface};
    use pierre_services::usage_counter::UsageCounterService;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt;

    use uuid::Uuid;

    /// Promotes a membership to `admin`; no repository method does.
    const PROMOTE: &str =
        "UPDATE tenant_users SET role = 'admin' WHERE user_id = $1 AND tenant_id = $2";

    /// The burst multiplier `UsageCounterService` applies when no admin
    /// override is set. `allowed` is `current < limit * multiplier`.
    const BURST_MULTIPLIER: f64 = 1.5;

    /// A read-only tool that needs no provider connection, so the only thing
    /// that can refuse the call is the quota ladder.
    const HARMLESS_TOOL: &str = "get_connection_status";

    async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId)> {
        common::init_server_config();
        common::init_test_http_clients();
        let resources = common::create_test_server_resources().await?;
        let email = format!("shared_quota_{}@example.com", Uuid::new_v4());
        let (user_id, _user) =
            common::create_test_user_with_email(&resources.agent.database, &email).await?;
        let tenants = resources.common.repos.tenants.get_all().await?;
        let tenant = tenants
            .iter()
            .find(|t| t.owner_user_id == user_id)
            .expect("a fresh user owns a tenant");
        Ok((Arc::clone(&resources), user_id, tenant.id))
    }

    /// Set the `counter` usage counter to exactly `target` for this user/tenant.
    async fn set_counter(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
        counter: &str,
        target: i64,
    ) {
        let svc = UsageCounterService::new(
            resources.common.repos.usage_counters.as_ref(),
            default_admin_config(),
        );
        let tenant = tenant_id.to_string();
        let user = user_id.to_string();
        let current = svc.get_current(&tenant, &user, counter).await.unwrap();
        let delta = target - current;
        assert!(delta >= 0, "the counter only moves forward in this test");
        if delta > 0 {
            svc.increment(&tenant, &user, counter, delta).await.unwrap();
        }
    }

    /// Run the chat surface's pre-turn check and report whether it refused,
    /// plus the limit it named.
    async fn chat_verdict(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Result<Option<(String, i64)>> {
        let ctx = resources.chat_pipeline_context();
        match check_pre_chat_quotas_scoped(&ctx, tenant_id, user_id, &PreChatScope::default()).await
        {
            Ok(_) => Ok(None),
            Err(e) => {
                assert_eq!(
                    e.code,
                    ErrorCode::QuotaExceeded,
                    "a cap breach must surface as QuotaExceeded, got {e:?}"
                );
                let details = e.details.as_deref().expect("quota errors carry details");
                Ok(Some((
                    details["limit_type"].as_str().unwrap().to_owned(),
                    details["limit"].as_i64().unwrap(),
                )))
            }
        }
    }

    /// Run a real `/mcp` tool dispatch and report whether the quota ladder
    /// refused it.
    async fn mcp_refused(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> bool {
        mcp_response(resources, user_id, tenant_id)
            .await
            .to_string()
            .contains("Rate limit exceeded")
    }

    /// The JSON-RPC response a real `/mcp` tool dispatch produces.
    async fn mcp_response(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Value {
        let state: Arc<dyn ToolRuntime> = Arc::clone(resources) as Arc<dyn ToolRuntime>;
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

        serde_json::to_value(&response).unwrap()
    }

    /// `POST /mcp` `tools/call` of [`HARMLESS_TOOL`] with the athlete's JWT,
    /// through the app the server serves, and the JSON-RPC body it answers.
    /// A refused tool call is still a served call: HTTP 200, answered in-band.
    async fn tools_call_over_http(resources: &Arc<ServerContext>, user_id: Uuid) -> Value {
        let user = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .expect("the athlete exists");
        let token = common::generate_test_token(resources, &user).await;
        let request = Request::post("/mcp")
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": { "name": HARMLESS_TOOL, "arguments": {} }
                })
                .to_string(),
            ))
            .unwrap();
        let response = ProviderToolRouter::build_http_app(resources)
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a refused tools/call is answered in-band, not as an HTTP error"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            body.get("error").is_none(),
            "a tool refusal is an isError result, not a JSON-RPC error: {body}"
        );
        body
    }

    /// The seconds from now to the next UTC midnight, when a daily counter
    /// resets, and that instant as the counter names it.
    fn to_next_utc_midnight() -> (i64, String) {
        let now = Utc::now();
        let tomorrow = (now + Duration::days(1)).date_naive();
        let midnight = tomorrow.and_hms_opt(0, 0, 0).unwrap().and_utc();
        (
            (midnight - now).num_seconds(),
            format!("{tomorrow}T00:00:00Z"),
        )
    }

    /// One below the hard limit, both doors are open; at the hard limit, both
    /// are shut. The flip happens at the same counter value because there is
    /// one ladder behind both.
    #[tokio::test]
    async fn mcp_and_chat_refuse_at_the_same_threshold() -> Result<()> {
        let (resources, user_id, tenant_id) = setup().await?;

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let hard_limit = (STARTER.daily_messages as f64 * BURST_MULTIPLIER) as i64;
        assert_eq!(
            hard_limit, 75,
            "Starter allows 50 daily messages with a 1.5x burst"
        );

        // One below the hard limit: both surfaces allow.
        set_counter(
            &resources,
            user_id,
            tenant_id,
            "daily_messages",
            hard_limit - 1,
        )
        .await;
        assert_eq!(
            chat_verdict(&resources, user_id, tenant_id).await?,
            None,
            "a chat turn is allowed at {} of {hard_limit}",
            hard_limit - 1
        );
        assert!(
            !mcp_refused(&resources, user_id, tenant_id).await,
            "an /mcp tool call is allowed at {} of {hard_limit}",
            hard_limit - 1
        );

        // At the hard limit: both surfaces refuse, and the chat side names the
        // same counter and the same number the shared policy read.
        set_counter(&resources, user_id, tenant_id, "daily_messages", hard_limit).await;
        assert_eq!(
            chat_verdict(&resources, user_id, tenant_id).await?,
            Some(("daily_messages".to_owned(), STARTER.daily_messages)),
            "a chat turn refuses at {hard_limit} against the Starter daily_messages cap"
        );
        assert!(
            mcp_refused(&resources, user_id, tenant_id).await,
            "an /mcp tool call refuses at the same {hard_limit}, not at a ladder of its own"
        );
        Ok(())
    }

    /// A daily refusal names its wait on both doors: the seconds to the next
    /// UTC midnight, attached once by the refusal itself, with the counter's
    /// own reset instant carried unchanged beside it.
    #[tokio::test]
    async fn daily_refusals_carry_the_seconds_to_the_reset() -> Result<()> {
        let (resources, user_id, tenant_id) = setup().await?;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let hard_limit = (STARTER.daily_messages as f64 * BURST_MULTIPLIER) as i64;
        set_counter(&resources, user_id, tenant_id, "daily_messages", hard_limit).await;

        let ctx = resources.chat_pipeline_context();
        let refusal =
            check_pre_chat_quotas_scoped(&ctx, tenant_id, user_id, &PreChatScope::default())
                .await
                .expect_err("the hard limit refuses a chat turn");
        let (to_midnight, resets_at) = to_next_utc_midnight();
        let details = refusal.details.as_deref().unwrap();
        assert_eq!(details["limit_type"], "daily_messages");
        assert_eq!(details["resets_at"].as_str(), Some(resets_at.as_str()));
        let chat_wait = refusal.retry_after_secs().unwrap();
        assert!(
            (i64::try_from(chat_wait).unwrap() - to_midnight).abs() <= 2,
            "chat refusal waits {chat_wait}s, midnight is {to_midnight}s away"
        );

        // The MCP door refuses from the same policy with the same wait: the
        // refusal its JSON-RPC error is built from carries it too.
        let inputs = QuotaPolicyInputs {
            repos: resources.common.repos.as_ref(),
            admin_config: resources
                .agent
                .admin_config
                .as_deref()
                .map(|c| c as &dyn AdminConfigLookup),
        };
        let mcp_refusal = check_quotas(&inputs, tenant_id, user_id, &QuotaSurface::McpToolCall)
            .await
            .expect_err("the hard limit refuses an MCP tool call");
        let details = mcp_refusal.details.as_deref().unwrap();
        assert_eq!(details["limit_type"], "daily_messages");
        assert_eq!(details["resets_at"].as_str(), Some(resets_at.as_str()));
        let mcp_wait = mcp_refusal.retry_after_secs().unwrap();
        assert!(
            (i64::try_from(mcp_wait).unwrap() - to_midnight).abs() <= 2,
            "MCP refusal waits {mcp_wait}s, midnight is {to_midnight}s away"
        );
        assert!(mcp_refused(&resources, user_id, tenant_id).await);
        Ok(())
    }

    /// The admin role used to walk past the `/mcp` ladder and never past the
    /// chat one. Sharing the policy removed that exemption: the only bypass is
    /// `QUOTA_BYPASS_USER_IDS`, which both surfaces honour.
    #[tokio::test]
    async fn admin_role_does_not_bypass_the_mcp_ladder() -> Result<()> {
        let (resources, user_id, tenant_id) = setup().await?;

        // Make the caller an admin of their own tenant — the exact condition
        // the old `/mcp` check short-circuited on. Written straight to
        // `tenant_users` because no repository method promotes a membership.
        match resources.agent.database.as_ref() {
            Database::SQLite(db) => {
                sqlx::query(PROMOTE)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .execute(db.pool())
                    .await?;
            }
            // `tenant_users` keys are `uuid` columns on PostgreSQL.
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                sqlx::query(PROMOTE)
                    .bind(user_id)
                    .bind(tenant_id.as_uuid())
                    .execute(db.pool())
                    .await?;
            }
        }
        let role = resources
            .common
            .repos
            .tenants
            .get_user_role(user_id, tenant_id)
            .await?;
        assert_eq!(
            role.as_deref(),
            Some("admin"),
            "the test caller must actually hold the admin role"
        );

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let hard_limit = (STARTER.daily_messages as f64 * BURST_MULTIPLIER) as i64;
        set_counter(&resources, user_id, tenant_id, "daily_messages", hard_limit).await;

        assert!(
            mcp_refused(&resources, user_id, tenant_id).await,
            "an admin is subject to the same caps on /mcp as on chat"
        );
        Ok(())
    }

    /// A spent tool-call budget reaches an MCP client as data it can back off
    /// on: the `isError` result of `POST /mcp` `tools/call` carries the
    /// refusal's counter, numbers, reset instant and wait as
    /// `structuredContent`, and the same JSON as a second text block for a
    /// client that reads only `content`. Rendering the refusal as its message
    /// alone left the client with "Rate limit exceeded" and no wait.
    #[tokio::test]
    async fn spent_tool_quota_reaches_the_mcp_client_as_structured_data() -> Result<()> {
        let (resources, user_id, tenant_id) = setup().await?;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let hard_limit = (STARTER.daily_tool_calls as f64 * BURST_MULTIPLIER) as i64;
        assert_eq!(
            hard_limit, 300,
            "Starter allows 200 daily tool calls with a 1.5x burst"
        );
        set_counter(
            &resources,
            user_id,
            tenant_id,
            "daily_tool_calls",
            hard_limit,
        )
        .await;

        let body = tools_call_over_http(&resources, user_id).await;
        let (to_midnight, resets_at) = to_next_utc_midnight();
        let result = &body["result"];
        assert_eq!(result["isError"], true, "the refusal is an error: {body}");

        let data = &result["structuredContent"];
        assert_eq!(data["limit_type"], "daily_tool_calls", "{body}");
        assert_eq!(data["limit"], STARTER.daily_tool_calls, "{body}");
        assert_eq!(data["current"], hard_limit, "{body}");
        assert_eq!(data["resets_at"].as_str(), Some(resets_at.as_str()));
        let wait = data["retry_after_secs"]
            .as_i64()
            .unwrap_or_else(|| panic!("the refusal names its wait: {body}"));
        assert!(wait >= 1, "a refusal in force never reads as retry now");
        assert!(
            (wait - to_midnight).abs() <= 2,
            "the refusal waits {wait}s, midnight is {to_midnight}s away"
        );

        let content = result["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "message, then the data as JSON: {body}");
        assert_eq!(content[0]["text"], "Rate limit exceeded");
        let text_data: Value = serde_json::from_str(content[1]["text"].as_str().unwrap())?;
        assert_eq!(
            &text_data, data,
            "the text block carries the same data as structuredContent"
        );
        Ok(())
    }

    /// A refusal that carries no data renders as its message alone: a tool
    /// disabled for the tenant answers `isError` with one text block and no
    /// `structuredContent`.
    #[tokio::test]
    async fn refusal_without_data_reaches_the_mcp_client_as_its_message() -> Result<()> {
        let (resources, user_id, tenant_id) = setup().await?;
        resources
            .mcp
            .tool_selection
            .set_tool_override(tenant_id, HARMLESS_TOOL, false, user_id, None)
            .await?;

        let body = tools_call_over_http(&resources, user_id).await;
        let result = &body["result"];
        assert_eq!(result["isError"], true, "the refusal is an error: {body}");
        assert!(
            result.get("structuredContent").is_none(),
            "no data, no structuredContent: {body}"
        );
        let content = result["content"].as_array().unwrap();
        assert_eq!(content.len(), 1, "the message alone: {body}");
        assert_eq!(
            content[0]["text"],
            format!(
                "Tool '{HARMLESS_TOOL}' is not available for your tenant. \
                 Contact your administrator to enable it."
            )
        );
        Ok(())
    }
}
