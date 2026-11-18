// ABOUTME: The data access request sweep sends each reminder stage once, even with two sweeps running at once
// ABOUTME: Nothing for a draft, an answered or a closed request; a moved due date is reminded again, in the reader's words

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    DataAccessLaw, DataAccessRequest, DataAccessScope, DataAccessStatus,
};
use pierre_database::RepositoryRegistry;
use pierre_notifications::{NotificationEvent, NotificationService, TenantId as CommereTenantId};
use pierre_services::data_access_request_sweep::{sweep, SweepOutcome};
use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
use pierre_test_support::db::{create_concurrent_test_db, create_test_db};
use tokio_util::task::TaskTracker;
use uuid::Uuid;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn morning(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(7, 0, 0).unwrap())
}

/// The notification service the server boots over the test database, with
/// the localizer that writes each reminder in the athlete's language.
fn notifications(db: &Database) -> NotificationService {
    let service = match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
        }
    };
    service.with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
        Arc::clone(db.repositories()),
        Arc::new(MessagingStringsRegistry::new()),
    )))
}

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

async fn insert(
    repos: &RepositoryRegistry,
    (user_id, tenant_id): (Uuid, TenantId),
    status: DataAccessStatus,
    due_on: NaiveDate,
) -> DataAccessRequest {
    let id = Uuid::new_v4().simple().to_string().to_uppercase();
    let at = morning(day(2026, 10, 1));
    let request = DataAccessRequest {
        id: Uuid::new_v4(),
        tenant_id,
        user_id,
        reference: format!("DRV-2026-{}", &id[..6]),
        controller: "Acme Fitness".to_owned(),
        controller_contact: "privacy@acme.example".to_owned(),
        athlete_identifier: "ana@example.com".to_owned(),
        law: DataAccessLaw::Gdpr,
        scope: DataAccessScope::Access,
        letter_locale: "fr".to_owned(),
        status,
        sent_on: Some(day(2026, 10, 21)),
        due_on: Some(due_on),
        extended_until: None,
        answered_on: None,
        due_soon_notified_at: None,
        overdue_notified_at: None,
        created_at: at,
        updated_at: at,
    };
    assert!(
        repos.data_access_requests.insert(&request).await.unwrap(),
        "bug: a fixture request wrote nothing"
    );
    request
}

/// The athlete's notifications of one event type, newest first.
async fn reminders(
    service: &NotificationService,
    (user_id, tenant_id): (Uuid, TenantId),
    event: NotificationEvent,
) -> Vec<(String, String)> {
    let (rows, _, _) = service
        .list_notifications(
            user_id,
            CommereTenantId(tenant_id.as_uuid()),
            50,
            0,
            None,
            false,
        )
        .await
        .unwrap();
    rows.into_iter()
        .filter(|row| row.notification_type == event.wire())
        .map(|row| (row.title, row.body))
        .collect()
}

#[tokio::test]
async fn two_sweeps_at_once_send_each_stage_once() {
    // Several connections, so the two sweeps' claims really overlap in the
    // engine rather than queueing behind one pooled connection.
    let db = create_concurrent_test_db().await.unwrap();
    let repos = db.repositories();
    let service = notifications(&db);
    let ana = athlete(repos, "ana").await;
    let ben = athlete(repos, "ben").await;
    let today = day(2026, 11, 18);
    insert(repos, ana, DataAccessStatus::Sent, day(2026, 11, 21)).await;
    insert(repos, ben, DataAccessStatus::Sent, day(2026, 11, 17)).await;

    let now = morning(today);
    let (first, second) = tokio::join!(
        sweep(repos, &service, today, now),
        sweep(repos, &service, today, now)
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(
        first.claimed + second.claimed,
        2,
        "bug: the two sweeps did not claim exactly the two due reminders between them: {first:?} {second:?}"
    );
    assert_eq!(
        first.sent + second.sent,
        2,
        "bug: a claimed reminder was not sent"
    );

    let due_soon = reminders(&service, ana, NotificationEvent::DataAccessRequestDueSoon).await;
    assert_eq!(
        due_soon.len(),
        1,
        "bug: Ana's due-soon reminder was sent {} times",
        due_soon.len()
    );
    let overdue = reminders(&service, ben, NotificationEvent::DataAccessRequestOverdue).await;
    assert_eq!(
        overdue.len(),
        1,
        "bug: Ben's overdue reminder was sent {} times",
        overdue.len()
    );
    assert!(
        reminders(&service, ana, NotificationEvent::DataAccessRequestOverdue)
            .await
            .is_empty(),
        "bug: a request not yet due was told it is overdue"
    );

    // The athlete's own language (the default, French), the due date written
    // out in it, the company and the reference.
    let (title, body) = &due_soon[0];
    assert!(
        title.contains("Acme Fitness"),
        "bug: the title does not name the company: {title}"
    );
    assert!(
        body.contains("21 novembre 2026") && body.contains("DRV-2026-"),
        "bug: the body does not carry the written due date and the reference: {body}"
    );
    let (_, overdue_body) = &overdue[0];
    assert!(
        overdue_body.contains("17 novembre 2026"),
        "bug: the overdue body does not carry its due date: {overdue_body}"
    );

    // The next day's pass finds nothing left to send for those stages.
    let next = sweep(
        repos,
        &service,
        day(2026, 11, 19),
        morning(day(2026, 11, 19)),
    )
    .await
    .unwrap();
    assert_eq!(
        next.claimed, 0,
        "bug: a stage was claimed again the next day: {next:?}"
    );
}

#[tokio::test]
async fn a_request_not_sent_or_already_settled_is_never_reminded() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let service = notifications(&db);
    let ana = athlete(repos, "ana").await;
    let today = day(2026, 11, 18);
    for status in [
        DataAccessStatus::Drafted,
        DataAccessStatus::Answered,
        DataAccessStatus::Closed,
    ] {
        insert(repos, ana, status, day(2026, 11, 19)).await;
        insert(repos, ana, status, day(2026, 11, 10)).await;
    }

    let outcome = sweep(repos, &service, today, morning(today)).await.unwrap();
    assert_eq!(
        outcome,
        SweepOutcome::default(),
        "bug: a draft, answered or closed request raised a reminder"
    );
    for event in [
        NotificationEvent::DataAccessRequestDueSoon,
        NotificationEvent::DataAccessRequestOverdue,
    ] {
        assert_eq!(
            reminders(&service, ana, event).await,
            Vec::new(),
            "bug: a settled request was reminded"
        );
    }
}

#[tokio::test]
async fn a_moved_due_date_is_reminded_again() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let service = notifications(&db);
    let ana = athlete(repos, "ana").await;
    let today = day(2026, 11, 18);
    let request = insert(repos, ana, DataAccessStatus::Sent, day(2026, 11, 20)).await;
    assert_eq!(
        sweep(repos, &service, today, morning(today))
            .await
            .unwrap()
            .sent,
        1,
        "bug: the first due date was not reminded"
    );

    // The controller notified an extension: the service moves the due date
    // and clears the stamps.
    let mut extended = repos
        .data_access_requests
        .get(request.tenant_id, request.user_id, request.id)
        .await
        .unwrap()
        .unwrap();
    extended.extended_until = Some(day(2027, 1, 20));
    extended.due_on = Some(day(2027, 1, 20));
    assert!(
        repos
            .data_access_requests
            .update(&extended, DataAccessStatus::Sent, true)
            .await
            .unwrap(),
        "bug: the extension could not be written"
    );

    let later = day(2027, 1, 18);
    assert_eq!(
        sweep(repos, &service, later, morning(later))
            .await
            .unwrap()
            .sent,
        1,
        "bug: the extended due date was not reminded"
    );
    let due_soon = reminders(&service, ana, NotificationEvent::DataAccessRequestDueSoon).await;
    assert_eq!(
        due_soon.len(),
        2,
        "bug: one reminder per due date, two due dates"
    );
    assert!(
        due_soon[0].1.contains("20 janvier 2027"),
        "bug: the second reminder names the old date: {}",
        due_soon[0].1
    );
}
