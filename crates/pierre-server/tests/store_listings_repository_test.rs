// ABOUTME: Direct StoreListingsRepository tests on whichever backend the test factory opens (SQLite or PostgreSQL)
// ABOUTME: Pins the review paths no route test reaches: ensure_listing, reject, the rejected list, admin stats, the install clamp
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The store routes exercise browse, search, install and uninstall over
//! HTTP, and the handle tests take an agent through submit and approve.
//! This file calls the trait itself for the rest of the review workflow —
//! a draft listing, a rejection, the rejected queue, the admin counts — and
//! for the install counter's floor at zero, so every value is pinned on both
//! drivers.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use pierre_core::models::agents::{
    AgentCategory, AgentVisibility, CreateSystemAgentRequest, PublishStatus,
};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::backends::StoreListingsRepository;
use uuid::Uuid;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

/// A system agent in `tenant_id` with no listing yet; returns its id as the
/// repository addresses it.
async fn create_agent(db: &Database, author_id: Uuid, tenant_id: TenantId, title: &str) -> String {
    let agent = db
        .repositories()
        .agents
        .create_system_agent(
            author_id,
            tenant_id,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: format!("You are the {title}."),
                category: AgentCategory::Training,
                tags: vec!["test".to_owned()],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    agent.id.to_string()
}

/// The install counter a published listing carries right now.
async fn installs(store: &dyn StoreListingsRepository, agent_id: &str) -> u32 {
    store
        .get_listing(agent_id)
        .await
        .unwrap()
        .expect("the published listing")
        .install_count
}

#[tokio::test]
async fn draft_listing_is_submitted_rejected_and_counted() {
    let db = create_test_db().await;
    let (author_id, tenant_id) = seed_user(&db).await;
    let admin_id = Uuid::new_v4();
    let repos = db.repositories();
    let store = &repos.store_listings;

    let agent_id = create_agent(&db, author_id, tenant_id, "Tempo Coach").await;
    assert!(store.get_listing(&agent_id).await.unwrap().is_none());

    // ensure_listing creates the draft once and then returns that same row.
    let draft = store.ensure_listing(&agent_id, tenant_id).await.unwrap();
    assert_eq!(draft.agent_id.to_string(), agent_id);
    assert_eq!(draft.tenant_id, tenant_id.to_string());
    assert_eq!(draft.publish_status, PublishStatus::Draft);
    assert_eq!(draft.install_count, 0);
    assert_eq!(draft.review_submitted_at, None);
    assert_eq!(draft.published_at, None);
    let again = store.ensure_listing(&agent_id, tenant_id).await.unwrap();
    assert_eq!(again.id, draft.id);

    assert!(store
        .get_pending_review_agents(tenant_id, None, None)
        .await
        .unwrap()
        .is_empty());

    // Submitting moves the existing draft into review in place.
    let pending = store
        .submit_for_review(&agent_id, author_id, tenant_id)
        .await
        .unwrap();
    assert_eq!(pending.id, draft.id);
    assert_eq!(pending.publish_status, PublishStatus::PendingReview);
    assert!(pending.review_submitted_at.is_some());
    assert!(pending.updated_at >= draft.updated_at);

    let queue = store
        .get_pending_review_agents(tenant_id, None, None)
        .await
        .unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].agent.id.to_string(), agent_id);
    assert_eq!(queue[0].agent.title, "Tempo Coach");
    assert_eq!(queue[0].listing.id, draft.id);
    assert_eq!(
        queue[0].listing.publish_status,
        PublishStatus::PendingReview
    );

    // Rejection records who decided, when, and why.
    let rejected = store
        .reject_agent(&agent_id, tenant_id, Some(admin_id), "Too vague to coach")
        .await
        .unwrap();
    assert_eq!(rejected.agent.id.to_string(), agent_id);
    assert_eq!(rejected.listing.publish_status, PublishStatus::Rejected);
    assert_eq!(
        rejected.listing.rejection_reason.as_deref(),
        Some("Too vague to coach")
    );
    assert_eq!(
        rejected.listing.review_decision_by.as_deref(),
        Some(admin_id.to_string().as_str())
    );
    assert!(rejected.listing.review_decision_at.is_some());
    assert_eq!(rejected.listing.published_at, None);

    // A decided listing cannot be decided again, nor resubmitted from rejected.
    assert!(store
        .reject_agent(&agent_id, tenant_id, Some(admin_id), "twice")
        .await
        .is_err());
    assert!(store
        .approve_agent(&agent_id, tenant_id, Some(admin_id))
        .await
        .is_err());
    assert!(store
        .submit_for_review(&agent_id, author_id, tenant_id)
        .await
        .is_err());

    let rejected_queue = store
        .get_rejected_agents(tenant_id, None, None)
        .await
        .unwrap();
    assert_eq!(rejected_queue.len(), 1);
    assert_eq!(rejected_queue[0].agent.id.to_string(), agent_id);
    assert_eq!(
        rejected_queue[0].listing.rejection_reason.as_deref(),
        Some("Too vague to coach")
    );
    assert!(store
        .get_pending_review_agents(tenant_id, None, None)
        .await
        .unwrap()
        .is_empty());

    let stats = store.get_store_admin_stats(tenant_id).await.unwrap();
    assert_eq!(stats.pending_count, 0);
    assert_eq!(stats.published_count, 0);
    assert_eq!(stats.rejected_count, 1);
    assert_eq!(stats.total_installs, 0);
    assert!((stats.rejection_rate - 100.0).abs() < f64::EPSILON);

    // A second agent approved alongside halves the rejection rate.
    let approved_id = create_agent(&db, author_id, tenant_id, "Hill Coach").await;
    store
        .submit_for_review(&approved_id, author_id, tenant_id)
        .await
        .unwrap();
    let approved = store
        .approve_agent(&approved_id, tenant_id, Some(admin_id))
        .await
        .unwrap();
    assert_eq!(approved.listing.publish_status, PublishStatus::Published);
    assert!(approved.listing.published_at.is_some());
    assert_eq!(approved.listing.rejection_reason, None);

    let stats = store.get_store_admin_stats(tenant_id).await.unwrap();
    assert_eq!(stats.pending_count, 0);
    assert_eq!(stats.published_count, 1);
    assert_eq!(stats.rejected_count, 1);
    assert!((stats.rejection_rate - 50.0).abs() < f64::EPSILON);

    // Another tenant's counts are its own.
    let (_, other_tenant) = seed_user(&db).await;
    let other = store.get_store_admin_stats(other_tenant).await.unwrap();
    assert_eq!(other.pending_count, 0);
    assert_eq!(other.published_count, 0);
    assert_eq!(other.rejected_count, 0);
    assert_eq!(other.total_installs, 0);
    assert!(other.rejection_rate.abs() < f64::EPSILON);
    assert!(store
        .get_rejected_agents(other_tenant, None, None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn install_count_never_drops_below_zero() {
    let db = create_test_db().await;
    let (author_id, tenant_id) = seed_user(&db).await;
    let repos = db.repositories();
    let store = &repos.store_listings;

    let agent_id = create_agent(&db, author_id, tenant_id, "Long Run Coach").await;
    store
        .submit_for_review(&agent_id, author_id, tenant_id)
        .await
        .unwrap();
    store
        .approve_agent(&agent_id, tenant_id, Some(author_id))
        .await
        .unwrap();

    assert_eq!(installs(store.as_ref(), &agent_id).await, 0);
    store.increment_install_count(&agent_id).await.unwrap();
    store.increment_install_count(&agent_id).await.unwrap();
    assert_eq!(installs(store.as_ref(), &agent_id).await, 2);
    assert_eq!(
        store
            .get_store_admin_stats(tenant_id)
            .await
            .unwrap()
            .total_installs,
        2
    );

    // Three decrements against two installs stop at the floor.
    store.decrement_install_count(&agent_id).await.unwrap();
    store.decrement_install_count(&agent_id).await.unwrap();
    assert_eq!(installs(store.as_ref(), &agent_id).await, 0);
    store.decrement_install_count(&agent_id).await.unwrap();
    assert_eq!(installs(store.as_ref(), &agent_id).await, 0);

    // The author's email resolves through the listing's author id; a
    // stranger's does not.
    let author = repos.users.get_global(author_id).await.unwrap().unwrap();
    assert_eq!(
        store.get_author_email(author_id).await.unwrap().as_deref(),
        Some(author.email.as_str())
    );
    assert_eq!(store.get_author_email(Uuid::new_v4()).await.unwrap(), None);
}
