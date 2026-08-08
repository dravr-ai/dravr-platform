// ABOUTME: Integration tests for offset pagination on GET /api/social/feed
// ABOUTME: Each page must carry different insights, and offset is the only pagination parameter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Pagination tests for the social feed route.
//!
//! The feed advances with `offset`, and the `next_cursor` it returns is the
//! offset the following page starts at. These tests seed a known set of
//! insights and walk the pages, asserting the ids are disjoint — a feed that
//! ignores the pagination parameter re-serves page one and fails here.

mod common;
mod helpers;

use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::{InsightType, ShareVisibility, SharedInsight};
use pierre_llm::LlmProvider;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_social::SocialRoutes;
use std::sync::Arc;
use uuid::Uuid;

/// Number of insights seeded before paginating.
const SEEDED_INSIGHTS: i64 = 5;

/// Test harness owning a server context and an authenticated user.
struct FeedPaginationSetup {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    jwt_token: String,
}

impl FeedPaginationSetup {
    async fn new() -> Self {
        common::init_server_config();
        let database = common::create_test_database()
            .await
            .expect("Failed to create test database");
        let auth_manager = common::create_test_auth_manager();
        let cache = common::create_test_cache()
            .await
            .expect("Failed to create test cache");

        let (user_id, user) = common::create_test_user(&database)
            .await
            .expect("Failed to create test user");

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config = Arc::new(ServerConfig {
            http_port: 8081,
            database: DatabaseConfig {
                url: DatabaseUrl::Memory,
                backup: BackupConfig {
                    directory: temp_dir.path().to_path_buf(),
                    ..Default::default()
                },
                ..Default::default()
            },
            app_behavior: AppBehaviorConfig {
                ci_mode: true,
                auto_approve_users: false,
                ..Default::default()
            },
            security: SecurityConfig {
                headers: SecurityHeadersConfig {
                    environment: Environment::Testing,
                },
                ..Default::default()
            },
            ..Default::default()
        });

        let llm_provider: Arc<dyn LlmProvider> = Arc::new(common::TestLlmProvider::valid());

        let resources = Arc::new(
            ServerContext::new(
                (*database).clone(),
                (*auth_manager).clone(),
                "test_jwt_secret",
                config,
                cache,
                ServerContextOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: Some(llm_provider),
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                },
            )
            .await,
        );

        let jwt_token = auth_manager
            .generate_token(&user, &resources.auth.jwks_manager)
            .expect("Failed to generate JWT");

        Self {
            resources,
            user_id,
            jwt_token,
        }
    }

    /// Seeds `SEEDED_INSIGHTS` insights owned by the test user, newest first.
    ///
    /// Timestamps are spaced a second apart from one instant so the feed's
    /// `created_at DESC` ordering is total and the expected page contents are
    /// fixed rather than dependent on insertion timing.
    async fn seed_insights(&self) -> Vec<String> {
        let social = self
            .resources
            .common
            .repos
            .social
            .clone()
            .expect("Social repository required for social tests");

        let base = Utc::now();
        let mut ids = Vec::new();

        for index in 0..SEEDED_INSIGHTS {
            let mut insight = SharedInsight::coach_generated(
                self.user_id,
                InsightType::Achievement,
                format!("Feed pagination insight {index}"),
                ShareVisibility::Public,
                format!("strava_activity_{index}"),
            );
            insight.created_at = base - Duration::seconds(index);
            insight.updated_at = insight.created_at;

            social
                .create_shared_insight(&insight)
                .await
                .expect("Failed to create insight");

            ids.push(insight.id.to_string());
        }

        ids
    }

    fn routes(&self) -> axum::Router {
        SocialRoutes::routes(self.resources.clone())
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.jwt_token)
    }

    /// Fetches a feed page and returns its decoded JSON body.
    async fn feed_page(&self, query: &str) -> serde_json::Value {
        let response = AxumTestRequest::get(&format!("/api/social/feed?{query}"))
            .header("authorization", &self.auth_header())
            .send(self.routes())
            .await;

        assert_eq!(
            response.status(),
            200,
            "feed request {query} should succeed"
        );
        response.json()
    }
}

/// Extracts the insight ids of a feed page in order.
fn insight_ids(body: &serde_json::Value) -> Vec<String> {
    body["items"]
        .as_array()
        .expect("feed items should be an array")
        .iter()
        .map(|item| {
            item["insight"]["id"]
                .as_str()
                .expect("insight id should be a string")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn test_feed_second_page_returns_different_insights() {
    let setup = FeedPaginationSetup::new().await;
    let seeded = setup.seed_insights().await;

    let page_one = setup.feed_page("limit=2").await;
    let page_two = setup.feed_page("limit=2&offset=2").await;

    let first_ids = insight_ids(&page_one);
    let second_ids = insight_ids(&page_two);

    assert_eq!(first_ids.len(), 2, "page one should hold two insights");
    assert_eq!(second_ids.len(), 2, "page two should hold two insights");

    // The seeded ids are ordered newest first, matching the feed's ordering.
    assert_eq!(first_ids, seeded[0..2].to_vec());
    assert_eq!(second_ids, seeded[2..4].to_vec());

    for id in &second_ids {
        assert!(
            !first_ids.contains(id),
            "insight {id} appeared on both page one and page two"
        );
    }
}

#[tokio::test]
async fn test_feed_next_cursor_advances_by_page_size() {
    let setup = FeedPaginationSetup::new().await;
    setup.seed_insights().await;

    let page_one = setup.feed_page("limit=2").await;
    assert_eq!(page_one["next_cursor"], "2");
    assert_eq!(page_one["has_more"], true);

    let page_two = setup.feed_page("limit=2&offset=2").await;
    assert_eq!(page_two["next_cursor"], "4");
    assert_eq!(page_two["has_more"], true);
}

#[tokio::test]
async fn test_feed_walks_every_insight_exactly_once() {
    let setup = FeedPaginationSetup::new().await;
    let seeded = setup.seed_insights().await;

    let mut collected = Vec::new();
    let mut offset = 0;

    loop {
        let page = setup.feed_page(&format!("limit=2&offset={offset}")).await;
        let ids = insight_ids(&page);
        if ids.is_empty() {
            break;
        }
        collected.extend(ids);
        offset += 2;

        assert!(
            offset <= 10,
            "pagination did not terminate; collected {collected:?}"
        );
    }

    assert_eq!(
        collected, seeded,
        "every insight should appear exactly once"
    );
}

#[tokio::test]
async fn test_feed_last_page_reports_no_more_items() {
    let setup = FeedPaginationSetup::new().await;
    setup.seed_insights().await;

    let last_page = setup.feed_page("limit=2&offset=4").await;

    assert_eq!(
        insight_ids(&last_page).len(),
        1,
        "five insights, pages of two"
    );
    assert_eq!(last_page["has_more"], false);
    assert_eq!(last_page["next_cursor"], serde_json::Value::Null);
}

#[tokio::test]
async fn test_feed_reads_offset_and_not_cursor() {
    let setup = FeedPaginationSetup::new().await;
    setup.seed_insights().await;

    let page_one = setup.feed_page("limit=2").await;
    let by_cursor = setup.feed_page("limit=2&cursor=2").await;

    // `offset` is the endpoint's only pagination parameter. A `cursor` query is
    // not a second spelling of it, so it leaves the response on page one.
    assert_eq!(
        insight_ids(&by_cursor),
        insight_ids(&page_one),
        "cursor is not a pagination parameter for this endpoint"
    );
}
