// ABOUTME: Regression tests for group snapshot tenant resolution (Fix A).
// ABOUTME: A member's snapshot must resolve under THEIR own connection tenant, not the requester's.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `fetch_member_snapshots` used to guess a single tenant per member relative to
//! the requester (prefer the requester's tenant, else the member's sole tenant,
//! else fall back to the requester's). A member whose Strava connection lives in
//! a tenant other than the group-host tenant then resolved to a tenant where
//! they had no `provider_connections` row, yielding an empty snapshot
//! ("0 activité, 0 km") AND an empty `needs_reauth` list — while the
//! cross-tenant connection-status check still reported "connected". These tests
//! pin the corrected behavior: snapshots resolve under the member's OWN
//! connection tenant (cross-tenant lookup), matching their 1-1 chat.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "tools-groups")]
mod snapshot_tenant_tests {
    use crate::common::create_test_server_resources;
    use chrono::{Duration, Utc};
    use pierre_core::models::{
        Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    /// A bare active user with a real `users` row (FK target for tenants).
    async fn seed_user(resources: &ServerContext, label: &str) -> Uuid {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            format!("{label}-{}@test.com", Uuid::new_v4()),
            password_hash,
            Some(format!("{label} Member")),
        );
        user.user_status = UserStatus::Active;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        user_id
    }

    /// Create a tenant OWNED by `owner_id`. `create` inserts the owner into
    /// `tenant_users`, so `tenants.list_for_user(owner_id)` then includes it —
    /// the mechanism that makes a member belong to multiple tenants.
    async fn create_tenant_owned_by(resources: &ServerContext, owner_id: Uuid) -> TenantId {
        let tenant_id = TenantId::generate();
        let now = Utc::now();
        let tenant = Tenant {
            id: tenant_id,
            name: "Snapshot Tenant".to_owned(),
            slug: format!("snap-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: owner_id,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();
        tenant_id
    }

    fn recent_ride(id: &str) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Utc::now() - Duration::days(2),
            7_200,
            "strava".to_owned(),
        )
        .distance_meters(60_000.0)
        .build()
    }

    /// The core regression: a member belongs to two tenants (A, C), their Strava
    /// connection + cached activities live in A, and the snapshot is requested
    /// with a fallback tenant B they are NOT in. The old code resolved to B
    /// (multiple memberships, none shared → fallback) and read nothing; the fix
    /// finds the connection in A and the snapshot is populated.
    #[tokio::test]
    async fn snapshot_resolves_under_members_own_connection_tenant() {
        let resources = create_test_server_resources().await.unwrap();
        let member = seed_user(&resources, "raphsnap").await;

        // Member is in two tenants, neither of which is the fallback below.
        let tenant_a = create_tenant_owned_by(&resources, member).await;
        let _tenant_c = create_tenant_owned_by(&resources, member).await;

        // Connection + cached activities live in tenant A only.
        resources
            .common
            .repos
            .provider_connections
            .register_connection(member, tenant_a, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(member, &tenant_a, "strava", &[recent_ride("ride-a-1")])
            .await
            .unwrap();

        // Fallback = the group-host/requester tenant the member is NOT in.
        let fallback_tenant = TenantId::generate();
        let runtime: Arc<dyn ToolRuntime> = resources.clone();

        let snapshots = fetch_member_snapshots(&runtime, &[member], fallback_tenant).await;

        assert_eq!(snapshots.len(), 1, "one member requested → one snapshot");
        let snap = &snapshots[0];
        assert!(
            snap.weekly_activity_count >= 1,
            "member's activity must surface from THEIR connection tenant, got weekly_activity_count={} (regression: empty snapshot under the wrong tenant)",
            snap.weekly_activity_count
        );
        assert!(
            snap.weekly_volume_km > 0.0,
            "weekly volume must be non-zero, got {}",
            snap.weekly_volume_km
        );
    }

    /// A dead connection must surface in `needs_reauth_providers` even when it
    /// lives in a tenant other than the fallback. The old tenant-scoped re-query
    /// against the fallback tenant returned nothing, so the coach never learned
    /// the member needed to reconnect.
    #[tokio::test]
    async fn needs_reauth_surfaces_across_tenants() {
        let resources = create_test_server_resources().await.unwrap();
        let member = seed_user(&resources, "reauthsnap").await;

        let tenant_a = create_tenant_owned_by(&resources, member).await;
        let _tenant_c = create_tenant_owned_by(&resources, member).await;

        resources
            .common
            .repos
            .provider_connections
            .register_connection(member, tenant_a, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(member, tenant_a, "strava", Some("invalid_grant"))
            .await
            .unwrap();

        let fallback_tenant = TenantId::generate();
        let runtime: Arc<dyn ToolRuntime> = resources.clone();

        let snapshots = fetch_member_snapshots(&runtime, &[member], fallback_tenant).await;

        assert_eq!(snapshots.len(), 1);
        assert!(
            snapshots[0]
                .needs_reauth_providers
                .iter()
                .any(|p| p == "strava"),
            "a dead connection in a non-fallback tenant must still be reported, got {:?}",
            snapshots[0].needs_reauth_providers
        );
    }

    /// A warm-but-stale cache whose bounded refresh cannot freshen anything
    /// (here: a second connected provider with no rows and no credentials)
    /// must serve the cached rows AND mark the snapshot `served_stale` — the
    /// signal the context renderer turns into a fetch-fresh-data directive.
    /// The rows themselves must survive: a refresh whose providers all fail
    /// must never replace real cached activities with emptiness.
    #[tokio::test]
    async fn stale_unrefreshable_cache_is_served_but_marked() {
        let resources = create_test_server_resources().await.unwrap();
        let member = seed_user(&resources, "stalesnap").await;
        let tenant_a = create_tenant_owned_by(&resources, member).await;

        // Strava: cached rows (warm). WHOOP: connected but rowless, which
        // makes the member's cache stale as a whole; with no stored token the
        // refresh attempt completes immediately without freshening it.
        for provider in ["strava", "whoop"] {
            resources
                .common
                .repos
                .provider_connections
                .register_connection(member, tenant_a, provider, &ConnectionType::OAuth, None)
                .await
                .unwrap();
        }
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(member, &tenant_a, "strava", &[recent_ride("ride-stale-1")])
            .await
            .unwrap();

        let fallback_tenant = TenantId::generate();
        let runtime: Arc<dyn ToolRuntime> = resources.clone();

        let snapshots = fetch_member_snapshots(&runtime, &[member], fallback_tenant).await;

        assert_eq!(snapshots.len(), 1);
        let snap = &snapshots[0];
        assert!(
            snap.served_stale,
            "an unrefreshable stale cache must mark the snapshot served_stale"
        );
        assert!(
            snap.weekly_activity_count >= 1,
            "the cached ride must still be served — a failed refresh must not \
             replace real rows with emptiness, got weekly_activity_count={}",
            snap.weekly_activity_count
        );
        assert!(
            snap.weekly_volume_km > 0.0,
            "cached volume must survive the failed refresh, got {}",
            snap.weekly_volume_km
        );
    }

    /// The control: a member whose every connected provider has fresh cached
    /// rows is NOT marked stale — the directive must never fire for members
    /// whose snapshot the turn can trust.
    #[tokio::test]
    async fn fresh_cache_is_not_marked_stale() {
        let resources = create_test_server_resources().await.unwrap();
        let member = seed_user(&resources, "freshsnap").await;
        let tenant_a = create_tenant_owned_by(&resources, member).await;

        resources
            .common
            .repos
            .provider_connections
            .register_connection(member, tenant_a, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(member, &tenant_a, "strava", &[recent_ride("ride-fresh-1")])
            .await
            .unwrap();

        let fallback_tenant = TenantId::generate();
        let runtime: Arc<dyn ToolRuntime> = resources.clone();

        let snapshots = fetch_member_snapshots(&runtime, &[member], fallback_tenant).await;

        assert_eq!(snapshots.len(), 1);
        assert!(
            !snapshots[0].served_stale,
            "a fully fresh cache must not carry the stale marker"
        );
        assert!(
            snapshots[0].weekly_activity_count >= 1,
            "the fresh ride must be served"
        );
    }
}
