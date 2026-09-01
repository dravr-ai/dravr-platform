// ABOUTME: One quota policy, two doors — /mcp and a chat turn refuse the same user at the same number
// ABOUTME: Driven through both real entry points, at the exact counter value where the verdict flips

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
    use dravr_tronc::mcp::tool::ToolContext;
    use pierre_chat_pipeline::quota_policy::{check_pre_chat_quotas_scoped, PreChatScope};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{TenantId, STARTER};
    use pierre_database::backends::factory::Database;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::mcp::tool_handlers::ToolHandlers;
    use pierre_runtime_context::default_admin_config;
    use pierre_services::usage_counter::UsageCounterService;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::json;
    use std::sync::Arc;

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
            common::create_test_user_with_email(&resources.coach.database, &email).await?;
        let tenants = resources.common.repos.tenants.get_all().await?;
        let tenant = tenants
            .iter()
            .find(|t| t.owner_user_id == user_id)
            .expect("a fresh user owns a tenant");
        Ok((Arc::clone(&resources), user_id, tenant.id))
    }

    /// Set `daily_messages` to exactly `target` for this user/tenant.
    async fn set_daily_messages(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
        target: i64,
    ) {
        let svc = UsageCounterService::new(
            resources.common.repos.usage_counters.as_ref(),
            default_admin_config(),
        );
        let tenant = tenant_id.to_string();
        let user = user_id.to_string();
        let current = svc
            .get_current(&tenant, &user, "daily_messages")
            .await
            .unwrap();
        let delta = target - current;
        assert!(delta >= 0, "the counter only moves forward in this test");
        if delta > 0 {
            svc.increment(&tenant, &user, "daily_messages", delta)
                .await
                .unwrap();
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
        let state: Arc<dyn ToolRuntime> = Arc::clone(resources) as Arc<dyn ToolRuntime>;
        let tool_context = ToolContext {
            user_id: Some(user_id.to_string()),
            tenant_id: Some(tenant_id.to_string()),
            auth_method: Some("jwt_bearer".to_owned()),
            request_id: Some(json!(1)),
            is_admin: false,
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

        let rendered = serde_json::to_string(&response).unwrap();
        rendered.contains("Rate limit exceeded")
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
        set_daily_messages(&resources, user_id, tenant_id, hard_limit - 1).await;
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
        set_daily_messages(&resources, user_id, tenant_id, hard_limit).await;
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

    /// The admin role used to walk past the `/mcp` ladder and never past the
    /// chat one. Sharing the policy removed that exemption: the only bypass is
    /// `QUOTA_BYPASS_USER_IDS`, which both surfaces honour.
    #[tokio::test]
    async fn admin_role_does_not_bypass_the_mcp_ladder() -> Result<()> {
        let (resources, user_id, tenant_id) = setup().await?;

        // Make the caller an admin of their own tenant — the exact condition
        // the old `/mcp` check short-circuited on. Written straight to
        // `tenant_users` because no repository method promotes a membership.
        match resources.coach.database.as_ref() {
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
        set_daily_messages(&resources, user_id, tenant_id, hard_limit).await;

        assert!(
            mcp_refused(&resources, user_id, tenant_id).await,
            "an admin is subject to the same caps on /mcp as on chat"
        );
        Ok(())
    }
}
