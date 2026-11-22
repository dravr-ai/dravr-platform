// ABOUTME: The admin audit trail on whichever backend DATABASE_URL names: paging across one instant, and an admin's deletion
// ABOUTME: A row tied with the page boundary is read on the next page; a deleted admin's rows stay without their caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Several admin writes can land in one instant (a bulk console action, a
//! coarse clock). A cursor on the timestamp alone asked for rows strictly older
//! than the boundary, so the tied rows past it were never shown.

use dravr_tronc::dates::Timestamp;
use pierre_core::admin::models::{DELETED_ACCOUNT_LABEL, DEVICE_CLI_SERVICE_PREFIX};
use pierre_core::models::User;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    AuditEventFilter, AuditOutcome, AuditSource, NewAuditEvent, SettledAuditEvent,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::account::delete_account;
use pierre_test_support::db::create_test_db;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::admin_tokens_test::account;

/// Move every row of `action` to one instant: the repository stamps `now`,
/// which no test can make two rows share.
async fn stamp_all(db: &Database, action: &str) {
    let tied = Timestamp::parse_from_rfc3339("2026-11-04T20:31:45Z").unwrap();
    let sql = "UPDATE audit_events SET timestamp = $1 WHERE event_type = $2";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(tied)
                .bind(action)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(tied)
                .bind(action)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn the_trail_pages_through_rows_that_share_the_boundary_instant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let action = format!("test.tied.{}", Uuid::new_v4().simple());
    let mut written = Vec::new();
    for n in 0..3 {
        let id = repos
            .admin_audit
            .record(&NewAuditEvent {
                action: action.clone(),
                summary: format!("tied row {n}"),
                outcome: AuditOutcome::Success,
                source: AuditSource::Console,
                actor_user_id: None,
                tenant_id: None,
                ip_address: None,
                user_agent: None,
                metadata: json!({}),
            })
            .await
            .unwrap();
        written.push(id);
    }
    stamp_all(&db, &action).await;

    let mut filter = AuditEventFilter {
        action_prefix: Some(action.clone()),
        ..AuditEventFilter::default()
    };
    let first = repos.admin_audit.list(&filter, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    let last = first.last().unwrap();
    filter.before = Some(last.occurred_at);
    filter.before_id = Some(last.id.clone());
    let second = repos.admin_audit.list(&filter, 2).await.unwrap();

    let mut seen: Vec<String> = first.iter().chain(&second).map(|e| e.id.clone()).collect();
    seen.sort_unstable();
    written.sort_unstable();
    assert_eq!(
        seen, written,
        "bug: the row sharing the boundary instant must be on the next page"
    );
}

/// What one trail row holds of its caller, read past the repository, which
/// does not return the address or the user agent: `(user_id, ip_address,
/// user_agent, metadata as text)`, `None` once the row is gone.
async fn caller_of(
    db: &Database,
    id: &str,
) -> Option<(Option<String>, Option<String>, Option<String>, String)> {
    let sql = "SELECT CAST(user_id AS TEXT) AS user_id, ip_address, user_agent, \
               CAST(metadata AS TEXT) AS metadata FROM audit_events WHERE id = $1";
    macro_rules! read {
        ($pool:expr) => {{
            let row = sqlx::query(sql)
                .bind(id)
                .fetch_optional($pool)
                .await
                .unwrap()?;
            Some((
                row.try_get("user_id").unwrap(),
                row.try_get("ip_address").unwrap(),
                row.try_get("user_agent").unwrap(),
                row.try_get("metadata").unwrap(),
            ))
        }};
    }
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => read!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => read!(pg.pool()),
    }
}

/// One trail row, written from `ADDRESS` with `AGENT`.
async fn written(
    repos: &RepositoryRegistry,
    source: AuditSource,
    actor: Option<Uuid>,
    metadata: serde_json::Value,
) -> String {
    repos
        .admin_audit
        .record(&NewAuditEvent {
            action: "user.suspend".to_owned(),
            summary: "Suspended an account".to_owned(),
            outcome: AuditOutcome::Success,
            source,
            actor_user_id: actor,
            tenant_id: None,
            ip_address: Some(ADDRESS.to_owned()),
            user_agent: Some(AGENT.to_owned()),
            metadata,
        })
        .await
        .unwrap()
}

const ADDRESS: &str = "203.0.113.9";
const AGENT: &str = "pierre-cli/0.3.0";

/// The audit trail follows the request log's rule (carnet#1589): the rows a
/// deleted admin wrote stay, with what they did and when, and lose who
/// wrote them: the address, the user agent, and the account inside the
/// metadata, where a console session names its admin by id and email and a
/// device login by email. A row another admin wrote about the deleted
/// account is purged, as before. Another admin's own rows keep everything.
#[tokio::test]
async fn a_deleted_admins_trail_rows_stay_without_their_caller() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving-admin@example.com", true).await;
    let staying = account(repos, "staying-admin@example.com", true).await;
    let athlete = Uuid::new_v4();
    let console_metadata = |admin: &User| {
        json!({
            "method": "POST",
            "route": "/api/admin/suspend-user/{user_id}",
            "status": 200,
            "params": { "user_id": athlete },
            "token_id": format!("cookie:{}", admin.id),
            "service": admin.email,
        })
    };

    let by_console = written(
        repos,
        AuditSource::Console,
        Some(leaving.id),
        console_metadata(&leaving),
    )
    .await;
    let by_cli = written(
        repos,
        AuditSource::Cli,
        Some(leaving.id),
        json!({
            "route": "/admin/suspend-user/{user_id}",
            "params": { "user_id": athlete },
            "token_id": "tok-1",
            "service": format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.email),
        }),
    )
    .await;
    let about_it = written(
        repos,
        AuditSource::Console,
        Some(staying.id),
        json!({ "target": { "user_id": leaving.id }, "service": staying.email }),
    )
    .await;
    let theirs = written(
        repos,
        AuditSource::Console,
        Some(staying.id),
        console_metadata(&staying),
    )
    .await;

    delete_account(repos, leaving.id, None).await.unwrap();

    for (what, id) in [("the console row", &by_console), ("the CLI row", &by_cli)] {
        let (actor, address, agent, metadata) = caller_of(&db, id)
            .await
            .unwrap_or_else(|| panic!("{what} stays"));
        assert_eq!(actor, None, "{what}: the actor goes with the account");
        assert_eq!(address, None, "{what}: and the address it wrote from");
        assert_eq!(agent, None, "{what}: and its user agent");
        assert!(
            !metadata.contains(&leaving.id.to_string()) && !metadata.contains(&leaving.email),
            "{what}: the metadata names the account no more: {metadata}"
        );
        assert!(
            metadata.contains(&athlete.to_string()) && metadata.contains(DELETED_ACCOUNT_LABEL),
            "{what}: what it did stays: {metadata}"
        );
    }
    let kept = repos
        .admin_audit
        .list(&AuditEventFilter::default(), 50)
        .await
        .unwrap();
    let console_row = kept.iter().find(|row| row.id == by_console).unwrap();
    assert_eq!(
        console_row.metadata["token_id"],
        format!("cookie:{DELETED_ACCOUNT_LABEL}")
    );
    assert_eq!(console_row.metadata["service"], DELETED_ACCOUNT_LABEL);
    let cli_row = kept.iter().find(|row| row.id == by_cli).unwrap();
    assert_eq!(
        cli_row.metadata["service"],
        format!("{DEVICE_CLI_SERVICE_PREFIX}{DELETED_ACCOUNT_LABEL}")
    );

    assert_eq!(
        caller_of(&db, &about_it).await,
        None,
        "a row about the deleted account is purged"
    );
    let (actor, address, agent, metadata) = caller_of(&db, &theirs).await.unwrap();
    assert_eq!(actor, Some(staying.id.to_string()));
    assert_eq!(address.as_deref(), Some(ADDRESS));
    assert_eq!(agent.as_deref(), Some(AGENT));
    assert!(metadata.contains(&staying.email), "{metadata}");
}

/// A started row is settled once, by the actor and tenant it names, and
/// nothing else changes it (EFVP 9.4d: an admin-only tool call is written
/// before it runs and settled to how it ended).
#[tokio::test]
async fn a_started_row_is_settled_once_and_only_under_its_actor_and_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let admin = account(repos, "settle-admin@example.com", true).await.id;
    let action = format!("tool.settle.{}", Uuid::new_v4().simple());
    // A platform-wide row: `tenant_id` references a tenant on SQLite, and the
    // tenant only has to differ for the settle below to miss.
    let tenant: Option<String> = None;
    let id = repos
        .admin_audit
        .record(&NewAuditEvent {
            action: action.clone(),
            summary: "admin_update_system_agent called over mcp_http".to_owned(),
            outcome: AuditOutcome::Started,
            source: AuditSource::ToolCall,
            actor_user_id: Some(admin),
            tenant_id: tenant.clone(),
            ip_address: None,
            user_agent: None,
            metadata: json!({ "transport": "mcp_http" }),
        })
        .await
        .unwrap();
    let outcome = || async {
        let rows = repos
            .admin_audit
            .list(
                &AuditEventFilter {
                    action_prefix: Some(action.clone()),
                    ..AuditEventFilter::default()
                },
                10,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "bug: the action names one row");
        (rows[0].outcome.clone(), rows[0].source.clone())
    };
    assert_eq!(
        outcome().await,
        ("started".to_owned(), "tool_call".to_owned()),
        "bug: a recorded tool call starts as started, sourced as a tool call"
    );

    let settle = |actor, tenant: Option<String>, outcome| {
        let trail = repos.admin_audit.clone();
        let id = id.clone();
        async move {
            trail
                .settle(&SettledAuditEvent {
                    id: &id,
                    actor_user_id: actor,
                    tenant_id: tenant.as_deref(),
                    outcome,
                })
                .await
                .unwrap()
        }
    };
    assert!(
        !settle(Some(Uuid::new_v4()), tenant.clone(), AuditOutcome::Failed).await,
        "bug: another actor settles nothing"
    );
    assert!(
        !settle(
            Some(admin),
            Some(Uuid::new_v4().to_string()),
            AuditOutcome::Failed
        )
        .await,
        "bug: another tenant settles nothing"
    );
    assert!(
        settle(Some(admin), tenant.clone(), AuditOutcome::Success).await,
        "bug: the actor and tenant that wrote the row settle it"
    );
    assert_eq!(
        outcome().await.0,
        "success",
        "bug: the row is settled to the outcome given"
    );
    assert!(
        !settle(Some(admin), tenant.clone(), AuditOutcome::Failed).await,
        "bug: a settled row is settled once"
    );
    assert_eq!(
        outcome().await.0,
        "success",
        "bug: a second settle leaves the first outcome"
    );
}
