// ABOUTME: Gives a seeded athlete a live Strava grant: the OAuth token row and the connection that reads it
// ABOUTME: For the suites whose reads go through Strava's API grant (history walks, segment history, the date goldens)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Only the suites a feature set compiles call it.
#![allow(dead_code)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use secrecy::SecretString;
use uuid::Uuid;

/// Give the athlete a live Strava grant and its connection.
pub(crate) async fn hold_strava_grant(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) {
    let now = Utc::now();
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: oauth_providers::STRAVA.to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: SecretString::from(format!("strava_access_token_{:0>30}", 7)),
            refresh_token: Some(SecretString::from("strava_refresh")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::STRAVA,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
}
