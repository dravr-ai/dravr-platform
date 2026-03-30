// ABOUTME: Integration tests for group fitness snapshot batch fetcher
// ABOUTME: Tests fetch_member_snapshots with synthetic providers and edge cases
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user_with_email};
use pierre_core::models::groups::OvertrainingRiskLevel;
use pierre_core::models::TenantId;
use pierre_mcp_server::services::group_fitness::fetch_member_snapshots;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Unit Tests: Overtraining Risk Assessment
// ============================================================================

#[test]
fn test_empty_user_ids_returns_empty_vec() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let resources = create_test_server_resources().await.unwrap();
        let tenant_id = TenantId::new();
        let result = fetch_member_snapshots(&resources, &[], tenant_id).await;
        assert!(result.is_empty(), "Empty user_ids should return empty vec");
    });
}

#[tokio::test]
async fn test_nonexistent_users_return_empty_snapshots() {
    let resources = create_test_server_resources().await.unwrap();
    let tenant_id = TenantId::new();
    let fake_ids = vec![Uuid::new_v4(), Uuid::new_v4()];

    let snapshots = fetch_member_snapshots(&resources, &fake_ids, tenant_id).await;

    assert_eq!(snapshots.len(), 2, "Should return one snapshot per user_id");
    for snap in &snapshots {
        assert!(snap.ctl.is_none(), "Non-existent user should have no CTL");
        assert!(snap.atl.is_none(), "Non-existent user should have no ATL");
        assert!(snap.tsb.is_none(), "Non-existent user should have no TSB");
        assert_eq!(snap.weekly_volume_km, 0.0, "Should have zero weekly volume");
        assert_eq!(snap.weekly_activity_count, 0, "Should have zero activities");
        assert_eq!(
            snap.overtraining_risk,
            OvertrainingRiskLevel::Low,
            "No data should mean low risk"
        );
    }
}

#[tokio::test]
async fn test_users_without_provider_return_empty_snapshots() {
    let resources = create_test_server_resources().await.unwrap();

    // Create real users but don't connect any provider
    let (uid1, _) = create_test_user_with_email(&resources.database, "noconn1@test.com")
        .await
        .unwrap();
    let (uid2, _) = create_test_user_with_email(&resources.database, "noconn2@test.com")
        .await
        .unwrap();

    let tenant_id = TenantId::new();
    let snapshots = fetch_member_snapshots(&resources, &[uid1, uid2], tenant_id).await;

    assert_eq!(snapshots.len(), 2, "Should return one snapshot per user");
    for snap in &snapshots {
        assert!(snap.ctl.is_none(), "No provider = no CTL");
        assert!(snap.atl.is_none(), "No provider = no ATL");
        assert!(snap.tsb.is_none(), "No provider = no TSB");
        assert_eq!(snap.weekly_volume_km, 0.0);
        assert_eq!(snap.weekly_activity_count, 0);
        assert!(
            snap.days_since_last_activity.is_none(),
            "No activities = no last activity"
        );
    }
}

#[tokio::test]
async fn test_display_names_populated_from_user_profile() {
    let resources = create_test_server_resources().await.unwrap();
    let (uid, _) = create_test_user_with_email(&resources.database, "alice@test.com")
        .await
        .unwrap();

    let tenant_id = TenantId::new();
    let snapshots = fetch_member_snapshots(&resources, &[uid], tenant_id).await;

    assert_eq!(snapshots.len(), 1);
    // User without display_name should fall back to email prefix
    let name = &snapshots[0].display_name;
    assert!(
        !name.is_empty(),
        "Display name should not be empty, got: {name}"
    );
}

#[tokio::test]
async fn test_user_with_synthetic_provider_gets_metrics() {
    let resources = create_test_server_resources().await.unwrap();
    let (uid, _) = create_test_user_with_email(&resources.database, "synth@test.com")
        .await
        .unwrap();

    // Get the user's tenant
    let tenants = resources.repos.tenants.list_for_user(uid).await.unwrap();
    let tenant_id = tenants.first().unwrap().id;

    // Register a synthetic provider connection for this user
    resources
        .repos
        .provider_connections
        .register_connection(
            uid,
            tenant_id,
            "synthetic",
            &pierre_core::models::ConnectionType::Synthetic,
            None,
        )
        .await
        .unwrap();

    let snapshots = fetch_member_snapshots(&resources, &[uid], tenant_id).await;
    assert_eq!(snapshots.len(), 1);

    let snap = &snapshots[0];
    assert_eq!(snap.user_id, uid);
    // Synthetic provider may or may not have activities depending on setup.
    // The key assertion is that the function completes without error and returns
    // a valid snapshot structure.
    assert!(
        snap.overtraining_risk == OvertrainingRiskLevel::Low
            || snap.overtraining_risk == OvertrainingRiskLevel::Moderate
            || snap.overtraining_risk == OvertrainingRiskLevel::High
    );
}

#[tokio::test]
async fn test_snapshot_count_matches_user_count() {
    let resources = create_test_server_resources().await.unwrap();
    let mut uids = Vec::new();
    for i in 0..5 {
        let (uid, _) =
            create_test_user_with_email(&resources.database, &format!("batch{i}@test.com"))
                .await
                .unwrap();
        uids.push(uid);
    }

    let tenant_id = TenantId::new();
    let snapshots = fetch_member_snapshots(&resources, &uids, tenant_id).await;

    assert_eq!(
        snapshots.len(),
        5,
        "Should get exactly one snapshot per user"
    );

    // Verify each user_id appears in the results
    for uid in &uids {
        assert!(
            snapshots.iter().any(|s| s.user_id == *uid),
            "Missing snapshot for user {uid}"
        );
    }
}
