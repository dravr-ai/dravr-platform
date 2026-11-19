// ABOUTME: The originals bundle state: scoped to the athlete and tenant, one building per athlete, finished only while building
// ABOUTME: And the sweep's two reads: building rows whose heartbeat stopped, and rows past their expiry

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::repositories::{BundleContents, BundleStatus, ExportBundle};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

async fn athlete(repos: &RepositoryRegistry, label: &str) -> (Uuid, TenantId) {
    let user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some(label.to_owned()),
    );
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        format!("{label} tenant"),
        format!("{label}-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    (user.id, tenant.id)
}

fn building(user_id: Uuid, tenant_id: TenantId, at: DateTime<Utc>, sealed: bool) -> ExportBundle {
    ExportBundle {
        id: Uuid::new_v4(),
        tenant_id,
        user_id,
        status: BundleStatus::Building,
        sealed,
        files: None,
        missing: None,
        damaged: None,
        byte_size: None,
        sha256: None,
        error: None,
        created_at: at,
        heartbeat_at: at,
        finished_at: None,
        expires_at: at + Duration::days(7),
    }
}

fn contents() -> BundleContents {
    BundleContents {
        files: 3,
        missing: 1,
        damaged: 0,
        byte_size: 4096,
        sha256: "ab".repeat(32),
    }
}

#[tokio::test]
async fn a_bundle_is_the_athletes_own_in_their_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, ana_tenant) = athlete(repos, "ana").await;
    let (ben, ben_tenant) = athlete(repos, "ben").await;
    let bundle = repos
        .user_export_bundles
        .insert_building(&building(ana, ana_tenant, Utc::now(), true))
        .await
        .unwrap();
    assert_eq!(
        bundle.status,
        BundleStatus::Building,
        "bug: an inserted bundle is building"
    );
    assert!(bundle.sealed, "bug: the sealed flag is kept");

    let bundles = &repos.user_export_bundles;
    assert!(
        bundles
            .get(ana_tenant, ana, bundle.id)
            .await
            .unwrap()
            .is_some(),
        "bug: Ana does not read her own bundle"
    );
    assert!(
        bundles
            .get(ben_tenant, ben, bundle.id)
            .await
            .unwrap()
            .is_none(),
        "bug: another athlete reads Ana's bundle by its id"
    );
    assert!(
        bundles
            .get(ben_tenant, ana, bundle.id)
            .await
            .unwrap()
            .is_none(),
        "bug: Ana's bundle is read in another tenant"
    );
    assert!(
        bundles.latest(ben_tenant, ben).await.unwrap().is_none(),
        "bug: Ben's latest bundle is Ana's"
    );
    assert!(
        !bundles.delete(ben_tenant, ben, bundle.id).await.unwrap(),
        "bug: another athlete deletes Ana's bundle"
    );
    assert!(
        !bundles
            .finish_ready(ben_tenant, ben, bundle.id, &contents(), Utc::now())
            .await
            .unwrap(),
        "bug: another athlete finishes Ana's bundle"
    );
}

#[tokio::test]
async fn a_second_build_while_one_runs_reads_back_the_one_building() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, tenant) = athlete(repos, "ana").await;
    let bundles = &repos.user_export_bundles;
    let first = bundles
        .insert_building(&building(ana, tenant, Utc::now(), false))
        .await
        .unwrap();
    let second = bundles
        .insert_building(&building(ana, tenant, Utc::now(), false))
        .await
        .unwrap();
    assert_eq!(
        second.id, first.id,
        "bug: a second building row was written for one athlete"
    );

    let expires = Utc::now() + Duration::days(7);
    assert!(
        bundles
            .finish_ready(tenant, ana, first.id, &contents(), expires)
            .await
            .unwrap(),
        "bug: a building bundle could not be finished ready"
    );
    let ready = bundles.get(tenant, ana, first.id).await.unwrap().unwrap();
    assert_eq!(
        ready.status,
        BundleStatus::Ready,
        "bug: a finished bundle reads ready"
    );
    assert_eq!(ready.files, Some(3), "bug: its file count is kept");
    assert_eq!(ready.missing, Some(1), "bug: its missing count is kept");
    assert_eq!(ready.byte_size, Some(4096), "bug: its size is kept");
    assert_eq!(
        ready.sha256.as_deref(),
        Some("ab".repeat(32).as_str()),
        "bug: its digest is kept"
    );
    assert!(
        ready.finished_at.is_some(),
        "bug: a finished bundle names when it finished"
    );
    assert!(
        !bundles
            .finish_failed(tenant, ana, first.id, "late")
            .await
            .unwrap(),
        "bug: a ready bundle is finished a second time"
    );

    let third = bundles
        .insert_building(&building(
            ana,
            tenant,
            Utc::now() + Duration::seconds(1),
            false,
        ))
        .await
        .unwrap();
    assert_ne!(third.id, first.id, "bug: once ready, a new build may start");
    assert_eq!(
        bundles.latest(tenant, ana).await.unwrap().unwrap().id,
        third.id,
        "bug: the latest bundle is the newest"
    );
}

#[tokio::test]
async fn the_sweep_reads_stale_builds_and_expired_bundles() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, ana_tenant) = athlete(repos, "ana").await;
    let (ben, ben_tenant) = athlete(repos, "ben").await;
    let bundles = &repos.user_export_bundles;
    let long_ago = Utc::now() - Duration::hours(2);
    let stale = bundles
        .insert_building(&building(ana, ana_tenant, long_ago, false))
        .await
        .unwrap();
    let live = bundles
        .insert_building(&building(ben, ben_tenant, Utc::now(), false))
        .await
        .unwrap();
    let cutoff = Utc::now() - Duration::minutes(10);
    let found: Vec<Uuid> = bundles
        .stale_building(cutoff)
        .await
        .unwrap()
        .iter()
        .map(|b| b.id)
        .collect();
    assert!(found.contains(&stale.id), "bug: a stopped build is found");
    assert!(!found.contains(&live.id), "bug: a live build is not stale");

    assert!(
        bundles
            .heartbeat(ana_tenant, ana, stale.id, Utc::now())
            .await
            .unwrap(),
        "bug: a building bundle's heartbeat did not move"
    );
    assert!(
        !bundles
            .stale_building(cutoff)
            .await
            .unwrap()
            .iter()
            .any(|b| b.id == stale.id),
        "bug: a heartbeat keeps a build alive"
    );

    assert!(
        bundles
            .finish_failed(ana_tenant, ana, stale.id, "instance died")
            .await
            .unwrap(),
        "bug: a building bundle could not be failed"
    );
    let in_a_week = Utc::now() + Duration::days(8);
    let expired: Vec<Uuid> = bundles
        .expired(in_a_week)
        .await
        .unwrap()
        .iter()
        .map(|b| b.id)
        .collect();
    assert!(
        expired.contains(&stale.id) && expired.contains(&live.id),
        "bug: the sweep misses a bundle past its expiry: {expired:?}"
    );
    assert!(
        bundles.expired(Utc::now()).await.unwrap().is_empty(),
        "bug: nothing expires before its time"
    );
    assert!(
        bundles.delete(ana_tenant, ana, stale.id).await.unwrap(),
        "bug: Ana's bundle row was not deleted"
    );
    assert!(
        bundles
            .get(ana_tenant, ana, stale.id)
            .await
            .unwrap()
            .is_none(),
        "bug: a deleted bundle still reads back"
    );
}
