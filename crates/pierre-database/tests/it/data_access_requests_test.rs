// ABOUTME: Data access requests: the athlete's own in their tenant, written only from the status read, references unique
// ABOUTME: And the sweep's read of due rows across tenants, with each reminder stage claimed once per due date

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::repositories::{
    DataAccessLaw, DataAccessRequest, DataAccessScope, DataAccessStatus, ReminderStage,
};
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

fn day(y: i32, m: u32, d: u32) -> CivilDate {
    CivilDate::from_ymd(y, m, d).unwrap()
}

fn noon(date: CivilDate) -> Timestamp {
    date.and_hms(12, 0, 0).unwrap().and_utc().unwrap()
}

/// A reference unique per test run, so suites sharing a database never meet.
fn reference() -> String {
    let id = Uuid::new_v4().simple().to_string().to_uppercase();
    format!("DRV-2026-{}", &id[..6])
}

fn drafted(user_id: Uuid, tenant_id: TenantId) -> DataAccessRequest {
    let at = noon(day(2026, 11, 1));
    DataAccessRequest {
        id: Uuid::new_v4(),
        tenant_id,
        user_id,
        reference: reference(),
        controller: "Acme Fitness".to_owned(),
        controller_contact: "privacy@acme.example".to_owned(),
        athlete_identifier: "ana@example.com".to_owned(),
        law: DataAccessLaw::QuebecP39_1,
        scope: DataAccessScope::AccessPortability,
        letter_locale: "fr".to_owned(),
        status: DataAccessStatus::Drafted,
        sent_on: None,
        due_on: None,
        extended_until: None,
        answered_on: None,
        due_soon_notified_at: None,
        overdue_notified_at: None,
        created_at: at,
        updated_at: at,
    }
}

fn sent(
    mut request: DataAccessRequest,
    sent_on: CivilDate,
    due_on: CivilDate,
) -> DataAccessRequest {
    request.status = DataAccessStatus::Sent;
    request.sent_on = Some(sent_on);
    request.due_on = Some(due_on);
    request
}

#[tokio::test]
async fn a_request_is_the_athletes_own_in_their_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, ana_tenant) = athlete(repos, "ana").await;
    let (ben, ben_tenant) = athlete(repos, "ben").await;
    let requests = &repos.data_access_requests;
    let request = drafted(ana, ana_tenant);
    assert!(
        requests.insert(&request).await.unwrap(),
        "bug: insert wrote nothing"
    );

    let read = requests
        .get(ana_tenant, ana, request.id)
        .await
        .unwrap()
        .expect("bug: Ana does not read her own request");
    assert_eq!(read, request, "bug: the row does not read back as written");
    assert_eq!(
        requests.list(ana_tenant, ana).await.unwrap(),
        vec![request.clone()],
        "bug: Ana's list is not her one request"
    );

    assert!(
        requests
            .get(ben_tenant, ben, request.id)
            .await
            .unwrap()
            .is_none(),
        "bug: another athlete reads Ana's request by its id"
    );
    assert!(
        requests
            .get(ben_tenant, ana, request.id)
            .await
            .unwrap()
            .is_none(),
        "bug: Ana's request is read in another tenant"
    );
    assert!(
        requests.list(ana_tenant, ben).await.unwrap().is_empty(),
        "bug: Ben lists Ana's request"
    );
    let mut stolen = request.clone();
    stolen.user_id = ben;
    stolen.controller = "Ben's edit".to_owned();
    assert!(
        !requests
            .update(&stolen, DataAccessStatus::Drafted, false)
            .await
            .unwrap(),
        "bug: another athlete edits Ana's request"
    );
    let mut elsewhere = request.clone();
    elsewhere.tenant_id = ben_tenant;
    assert!(
        !requests
            .update(&elsewhere, DataAccessStatus::Drafted, false)
            .await
            .unwrap(),
        "bug: Ana's request is edited from another tenant"
    );
    assert_eq!(
        requests
            .get(ana_tenant, ana, request.id)
            .await
            .unwrap()
            .unwrap()
            .controller,
        "Acme Fitness",
        "bug: a refused edit changed the row"
    );
}

#[tokio::test]
async fn a_reference_is_written_once() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, tenant) = athlete(repos, "ana").await;
    let requests = &repos.data_access_requests;
    let first = drafted(ana, tenant);
    assert!(
        requests.insert(&first).await.unwrap(),
        "bug: the first request with a reference wrote nothing"
    );
    let mut second = drafted(ana, tenant);
    second.reference.clone_from(&first.reference);
    assert!(
        !requests.insert(&second).await.unwrap(),
        "bug: a second request took a reference already used"
    );
    assert_eq!(
        requests.list(tenant, ana).await.unwrap().len(),
        1,
        "bug: the refused insert wrote a row"
    );
}

#[tokio::test]
async fn an_update_writes_only_from_the_status_it_read() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, tenant) = athlete(repos, "ana").await;
    let requests = &repos.data_access_requests;
    let request = drafted(ana, tenant);
    requests.insert(&request).await.unwrap();

    let marked = sent(request.clone(), day(2026, 11, 2), day(2026, 12, 2));
    assert!(
        requests
            .update(&marked, DataAccessStatus::Drafted, true)
            .await
            .unwrap(),
        "bug: a draft could not be marked sent"
    );
    let mut late_edit = request.clone();
    late_edit.controller = "Renamed".to_owned();
    assert!(
        !requests
            .update(&late_edit, DataAccessStatus::Drafted, false)
            .await
            .unwrap(),
        "bug: a draft edit landed on a request already sent"
    );
    let read = requests
        .get(tenant, ana, request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.status,
        DataAccessStatus::Sent,
        "bug: the sent write did not land"
    );
    assert_eq!(
        read.sent_on,
        Some(day(2026, 11, 2)),
        "bug: the sending date did not land"
    );
    assert_eq!(
        read.due_on,
        Some(day(2026, 12, 2)),
        "bug: the due date did not land"
    );
    assert_eq!(
        read.controller, "Acme Fitness",
        "bug: the late draft edit changed the company"
    );
}

#[tokio::test]
async fn the_sweep_reads_each_stage_in_its_window_across_tenants() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, ana_tenant) = athlete(repos, "ana").await;
    let (ben, ben_tenant) = athlete(repos, "ben").await;
    let requests = &repos.data_access_requests;
    let today = day(2026, 11, 18);

    // Due in three days (in the window), due in four (outside it), due
    // yesterday (overdue), and an answered one past due (neither).
    let in_window = sent(
        drafted(ana, ana_tenant),
        day(2026, 10, 22),
        day(2026, 11, 21),
    );
    let later = sent(
        drafted(ben, ben_tenant),
        day(2026, 10, 23),
        day(2026, 11, 22),
    );
    let overdue = sent(
        drafted(ben, ben_tenant),
        day(2026, 10, 18),
        day(2026, 11, 17),
    );
    let mut answered = sent(
        drafted(ana, ana_tenant),
        day(2026, 10, 1),
        day(2026, 10, 31),
    );
    answered.status = DataAccessStatus::Answered;
    answered.answered_on = Some(day(2026, 11, 1));
    for request in [&in_window, &later, &overdue, &answered] {
        assert!(
            requests.insert(request).await.unwrap(),
            "bug: a fixture request wrote nothing"
        );
    }

    let ours = |rows: Vec<DataAccessRequest>| -> Vec<Uuid> {
        let mine = [in_window.id, later.id, overdue.id, answered.id];
        rows.into_iter()
            .map(|r| r.id)
            .filter(|id| mine.contains(id))
            .collect()
    };
    assert_eq!(
        ours(
            requests
                .due_for_reminder(ReminderStage::DueSoon, today, 500)
                .await
                .unwrap()
        ),
        vec![in_window.id],
        "bug: the due-soon read is not exactly the request due within three days"
    );
    assert_eq!(
        ours(
            requests
                .due_for_reminder(ReminderStage::Overdue, today, 500)
                .await
                .unwrap()
        ),
        vec![overdue.id],
        "bug: the overdue read is not exactly the sent request past due"
    );
}

#[tokio::test]
async fn a_reminder_is_claimed_once_per_due_date() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (ana, tenant) = athlete(repos, "ana").await;
    let requests = &repos.data_access_requests;
    let request = sent(drafted(ana, tenant), day(2026, 10, 22), day(2026, 11, 21));
    requests.insert(&request).await.unwrap();
    let due = day(2026, 11, 21);
    let at = noon(day(2026, 11, 18));

    assert!(
        requests
            .claim_reminder(ReminderStage::DueSoon, &request, due, at)
            .await
            .unwrap(),
        "bug: the first claim did not take the stage"
    );
    assert!(
        !requests
            .claim_reminder(ReminderStage::DueSoon, &request, due, at)
            .await
            .unwrap(),
        "bug: the same stage was claimed twice"
    );
    assert!(
        requests
            .due_for_reminder(ReminderStage::DueSoon, day(2026, 11, 18), 500)
            .await
            .unwrap()
            .iter()
            .all(|r| r.id != request.id),
        "bug: a claimed stage is read as due again"
    );
    let read = requests
        .get(tenant, ana, request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.due_soon_notified_at,
        Some(at),
        "bug: the claim did not stamp the due-soon stage"
    );
    assert_eq!(
        read.overdue_notified_at, None,
        "bug: one claim stamped both stages"
    );

    // An edit that keeps the due date keeps the stamp; one that moves it
    // clears both, so the new date is reminded once again.
    assert!(
        requests
            .update(&read, DataAccessStatus::Sent, false)
            .await
            .unwrap(),
        "bug: an edit keeping the due date was refused"
    );
    assert_eq!(
        requests
            .get(tenant, ana, request.id)
            .await
            .unwrap()
            .unwrap()
            .due_soon_notified_at,
        Some(at),
        "bug: an edit that kept the due date lost the claim"
    );
    let mut extended = read.clone();
    extended.extended_until = Some(day(2027, 1, 21));
    extended.due_on = Some(day(2027, 1, 21));
    assert!(
        requests
            .update(&extended, DataAccessStatus::Sent, true)
            .await
            .unwrap(),
        "bug: an edit moving the due date was refused"
    );
    assert!(
        !requests
            .claim_reminder(ReminderStage::DueSoon, &request, due, at)
            .await
            .unwrap(),
        "bug: a claim for the old due date landed after the date moved"
    );
    assert!(
        requests
            .claim_reminder(ReminderStage::DueSoon, &request, day(2027, 1, 21), at)
            .await
            .unwrap(),
        "bug: the moved due date cannot be reminded"
    );

    // A request no longer sent is never claimed.
    let mut closed = requests
        .get(tenant, ana, request.id)
        .await
        .unwrap()
        .unwrap();
    closed.status = DataAccessStatus::Closed;
    assert!(
        requests
            .update(&closed, DataAccessStatus::Sent, false)
            .await
            .unwrap(),
        "bug: a sent request could not be closed"
    );
    assert!(
        !requests
            .claim_reminder(ReminderStage::Overdue, &request, day(2027, 1, 21), at)
            .await
            .unwrap(),
        "bug: a closed request was claimed"
    );
}
