// ABOUTME: Integration tests for the agent followup scheduler — overdue rows fire and transition delivered
// ABOUTME: Proves due_at metadata becomes real: tick processes overdue rows once, skips fresh ones, and leaves a row whose push failed pending
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, Utc};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::InsertAgentFollowupParams;
use pierre_services::agent_followup_scheduler::tick;
use uuid::Uuid;

/// Open the database the lane names through the test factory.
async fn open_db() -> Result<Database> {
    Ok(create_test_db().await?)
}

/// Seed the user, tenant, and agent rows the `agent_followups` foreign keys
/// resolve against, through the repositories so both backends accept them.
async fn seed_user_tenant_agent(db: &Database) -> Result<(TenantId, String, String)> {
    let repos = db.repositories();
    let user = User::new(
        format!("{}@test.local", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Test User".to_owned()),
    );
    repos.users.create(&user).await?;
    let tenant = Tenant::new(
        "Test Tenant".to_owned(),
        format!("tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await?;
    let agent = repos
        .agents
        .create(
            user.id,
            tenant.id,
            &CreateAgentRequest {
                title: "Test Coach".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await?;
    Ok((tenant.id, user.id.to_string(), agent.id.to_string()))
}

#[tokio::test]
async fn tick_processes_overdue_followup_and_marks_delivered() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;

    let due_in_past = Utc::now() - Duration::hours(1);
    let inserted = memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            agent_id: &agent_id,
            conversation_id: None,
            content: "check Achilles after 24h",
            due_at: Some(due_in_past),
        })
        .await?;
    assert_eq!(inserted.status.as_str(), "pending");

    // Tick at "now" — the row's due_at is in the past so it should fire.
    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;

    assert_eq!(
        outcome.processed, 1,
        "one overdue followup should be picked up"
    );
    assert_eq!(
        outcome.marked_delivered, 1,
        "row should transition to delivered"
    );
    assert_eq!(
        outcome.errors, 0,
        "no errors expected without notification service"
    );

    // Re-list pending — the delivered row should not show up.
    let pending = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert!(
        pending.is_empty(),
        "delivered followup should leave the pending queue"
    );

    Ok(())
}

#[tokio::test]
async fn tick_skips_followups_with_due_at_in_the_future() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;

    memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            agent_id: &agent_id,
            conversation_id: None,
            content: "future check",
            due_at: Some(Utc::now() + Duration::hours(2)),
        })
        .await?;

    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 0);
    assert_eq!(outcome.marked_delivered, 0);

    // The row is still pending after the tick.
    let pending = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert_eq!(pending.len(), 1);

    Ok(())
}

#[tokio::test]
async fn tick_skips_followups_with_no_due_at() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;

    memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            agent_id: &agent_id,
            conversation_id: None,
            content: "no specific time",
            due_at: None,
        })
        .await?;

    // due_at IS NULL → never picked up by the scheduler. The in-prompt
    // injection path handles those followups instead.
    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 0);

    let pending = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "no specific time");

    Ok(())
}

#[tokio::test]
async fn second_tick_does_not_re_process_delivered_row() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;

    memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            agent_id: &agent_id,
            conversation_id: None,
            content: "check once",
            due_at: Some(Utc::now() - Duration::minutes(10)),
        })
        .await?;

    let first = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(first.processed, 1);

    let second = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(
        second.processed, 0,
        "second tick must not re-pick a delivered row"
    );

    Ok(())
}

#[tokio::test]
async fn tick_processes_multiple_overdue_in_one_batch() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;

    for i in 0..5 {
        memory
            .insert_agent_followup(&InsertAgentFollowupParams {
                tenant_id: tenant,
                user_id: &user_id,
                agent_id: &agent_id,
                conversation_id: None,
                content: "overdue",
                due_at: Some(Utc::now() - Duration::minutes(30 + i)),
            })
            .await?;
    }

    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 5);
    assert_eq!(outcome.marked_delivered, 5);
    assert_eq!(outcome.errors, 0);

    Ok(())
}

/// What the dispatch outcome does to the row: a push that reached the
/// pipeline marks the followup delivered, a push that did not leaves it
/// pending so the next tick retries it (carnet#464 — the scheduler used to
/// mark a row delivered whatever the dispatch did, so a permanently failing
/// push was recorded as a check-in the athlete had received).
#[cfg(feature = "client-notifications")]
mod dispatch_outcome {
    use super::{open_db, seed_user_tenant_agent, Result};
    use chrono::{Duration, Utc};
    use pierre_database::backends::factory::Database;
    use pierre_database::repositories::InsertAgentFollowupParams;
    use pierre_memory::FollowupStatus;
    use pierre_notifications::{NotificationService, TenantId as CommTenantId};
    use pierre_services::agent_followup_scheduler::tick;
    use sqlx::sqlite::SqlitePoolOptions;
    use uuid::Uuid;

    /// A notification service whose store has no tables at all: every
    /// dispatch fails on the preference read, which is exactly what the
    /// scheduler sees when the pipeline's database is unreachable.
    async fn unreachable_notification_service() -> Result<NotificationService> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        Ok(NotificationService::from_sqlite(pool))
    }

    /// The notification service on the scheduler's own database, where the
    /// notification tables exist and a dispatch persists a row.
    fn reachable_notification_service(db: &Database) -> NotificationService {
        match db {
            Database::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
        }
    }

    #[tokio::test]
    async fn failed_dispatch_leaves_followup_pending_for_the_next_tick() -> Result<()> {
        let db = open_db().await?;
        let memory = db.repositories().memory;
        let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;
        let service = unreachable_notification_service().await?;

        let inserted = memory
            .insert_agent_followup(&InsertAgentFollowupParams {
                tenant_id: tenant,
                user_id: &user_id,
                agent_id: &agent_id,
                conversation_id: None,
                content: "ask how the Achilles held up",
                due_at: Some(Utc::now() - Duration::minutes(5)),
            })
            .await?;

        let first = tick(memory.as_ref(), Some(&service), Utc::now(), 100).await?;
        assert_eq!(first.processed, 1, "the overdue row is picked up");
        assert_eq!(first.dispatched, 1, "one push was attempted");
        assert_eq!(first.errors, 1, "the attempt failed");
        assert_eq!(
            first.marked_delivered, 0,
            "a failed push must not mark the row delivered"
        );

        let pending = memory
            .list_pending_followups_for_tenant(tenant, 100)
            .await?;
        assert_eq!(pending.len(), 1, "the row is still pending");
        assert_eq!(pending[0].id, inserted.id);
        assert_eq!(pending[0].status, FollowupStatus::Pending);
        assert_eq!(pending[0].delivered_at, None);

        // The next tick retries it — same row, same failure, still pending.
        let second = tick(memory.as_ref(), Some(&service), Utc::now(), 100).await?;
        assert_eq!(second.processed, 1, "the next tick retries the row");
        assert_eq!(second.dispatched, 1);
        assert_eq!(second.errors, 1);
        assert_eq!(second.marked_delivered, 0);

        let still_pending = memory
            .list_pending_followups_for_tenant(tenant, 100)
            .await?;
        assert_eq!(still_pending.len(), 1);
        assert_eq!(still_pending[0].id, inserted.id);

        Ok(())
    }

    #[tokio::test]
    async fn successful_dispatch_marks_followup_delivered_and_persists_the_push() -> Result<()> {
        let db = open_db().await?;
        let memory = db.repositories().memory;
        let (tenant, user_id, agent_id) = seed_user_tenant_agent(&db).await?;
        let service = reachable_notification_service(&db);

        memory
            .insert_agent_followup(&InsertAgentFollowupParams {
                tenant_id: tenant,
                user_id: &user_id,
                agent_id: &agent_id,
                conversation_id: None,
                content: "check on the taper week",
                due_at: Some(Utc::now() - Duration::minutes(5)),
            })
            .await?;

        let outcome = tick(memory.as_ref(), Some(&service), Utc::now(), 100).await?;
        assert_eq!(outcome.processed, 1);
        assert_eq!(outcome.dispatched, 1);
        assert_eq!(outcome.errors, 0);
        assert_eq!(
            outcome.marked_delivered, 1,
            "a push that reached the pipeline marks the row delivered"
        );

        let pending = memory
            .list_pending_followups_for_tenant(tenant, 100)
            .await?;
        assert!(
            pending.is_empty(),
            "the delivered row left the pending queue"
        );

        let user_uuid: Uuid = user_id.parse()?;
        let (notifications, total, _unread) = service
            .list_notifications(
                user_uuid,
                CommTenantId(tenant.as_uuid()),
                10,
                0,
                None,
                false,
            )
            .await?;
        assert_eq!(total, 1, "exactly one notification row was persisted");
        assert_eq!(notifications[0].notification_type, "coach_followup_due");
        assert_eq!(notifications[0].body, "check on the taper week");

        Ok(())
    }
}
