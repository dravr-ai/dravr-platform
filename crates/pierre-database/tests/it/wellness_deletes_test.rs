// ABOUTME: Covers the deletes a re-read source needs: nights overlapping a window, days in a date span, self-reports by id
// ABOUTME: Each scoped to one tenant, user and provider; a night touching the window only at an edge is kept

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `create_test_db` opens whichever `DATABASE_URL` names, so these run on
//! `SQLite` and `PostgreSQL` alike.

use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use pierre_core::models::{
    DataSource, DeviceType, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
    TenantId, User,
};
use pierre_database::repositories::{
    RecordFiling, RecordWindow, SelfReportWrite, WellnessWriteMode,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

const TENANT: &str = "00000000-0000-4000-8000-000000000011";
const OTHER_TENANT: &str = "00000000-0000-4000-8000-000000000012";
const KEY: &str = "terra";
const OTHER_KEY: &str = "garmin";

async fn user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("wellness-deletes-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Wellness Deletes Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

async fn data_source(
    repos: &RepositoryRegistry,
    tenant: &TenantId,
    user: Uuid,
    provider: &str,
) -> String {
    repos
        .data_sources
        .upsert_data_source(
            tenant,
            &DataSource {
                id: String::new(),
                user_id: user.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap()
}

fn at(hour: i64) -> Timestamp {
    CivilDate::from_ymd(2026, 11, 20)
        .unwrap()
        .at_midnight()
        .and_utc()
        .unwrap()
        + Elapsed::hours(hour)
}

/// Store a night from `start` to `end` (hours past the base day) for `user`
/// under `provider` in `tenant`.
async fn night(
    repos: &RepositoryRegistry,
    (tenant, user, provider): (&TenantId, Uuid, &str),
    (start, end): (i64, i64),
) {
    let source = data_source(repos, tenant, user, provider).await;
    let session = StoredSleepSession {
        id: String::new(),
        user_id: user.to_string(),
        data_source_id: source,
        is_nap: false,
        start_datetime: at(start),
        end_datetime: at(end),
        total_sleep_seconds: Some(3_600),
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: None,
        sleep_efficiency: None,
        avg_heart_rate: None,
        min_heart_rate: None,
        avg_hrv: None,
        sleep_score: None,
        avg_spo2: None,
        stages: Vec::new(),
        source_name: provider.to_owned(),
    };
    repos
        .sleep
        .upsert_sleep_session(tenant, &session, Timestamp::now())
        .await
        .unwrap()
        .unwrap();
}

/// The starts (hours past the base day) of the nights `user` holds in
/// `tenant`, under any provider, sorted.
async fn night_starts(repos: &RepositoryRegistry, tenant: &TenantId, user: Uuid) -> Vec<i64> {
    let mut starts: Vec<i64> = repos
        .sleep
        .get_sleep_sessions(user, tenant, at(-48), at(96))
        .await
        .unwrap()
        .iter()
        .map(|night| (night.start_datetime - at(0)).num_hours())
        .collect();
    starts.sort_unstable();
    starts
}

#[tokio::test]
async fn a_window_delete_removes_only_the_overlapping_nights_of_its_owner_and_key() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let other_tenant = TenantId::parse_str(OTHER_TENANT).unwrap();
    let (who, neighbour) = (user(repos).await, user(repos).await);
    let mine = (&tenant, who, KEY);
    // The window is [12, 36): noon to the next noon.
    night(repos, mine, (-2, 6)).await; // ends before it: kept
    night(repos, mine, (4, 12)).await; // ends at its start: kept
    night(repos, mine, (10, 14)).await; // crosses its start: deleted
    night(repos, mine, (22, 30)).await; // inside it: deleted
    night(repos, mine, (34, 40)).await; // crosses its end: deleted
    night(repos, mine, (36, 42)).await; // starts at its end: kept
    night(repos, (&tenant, who, OTHER_KEY), (23, 31)).await;
    night(repos, (&tenant, neighbour, KEY), (24, 32)).await;
    night(repos, (&other_tenant, who, KEY), (25, 33)).await;

    let deleted = repos
        .sleep
        .delete_sleep_sessions_overlapping(who, &tenant, KEY, (at(12), at(36)))
        .await
        .unwrap();

    assert_eq!(deleted, 3, "bug: the nights overlapping the window");
    assert_eq!(
        night_starts(repos, &tenant, who).await,
        vec![-2, 4, 23, 36],
        "bug: a night outside the window, at its edge or under another key was deleted"
    );
    assert_eq!(
        night_starts(repos, &tenant, neighbour).await,
        vec![24],
        "bug: another user's night was deleted"
    );
    assert_eq!(
        night_starts(repos, &other_tenant, who).await,
        vec![25],
        "bug: the user's night in another tenant was deleted"
    );
}

#[tokio::test]
async fn a_window_spanning_a_whole_night_deletes_it_and_a_window_beside_it_deletes_nothing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let who = user(repos).await;
    night(repos, (&tenant, who, KEY), (20, 30)).await;

    let none = repos
        .sleep
        .delete_sleep_sessions_overlapping(who, &tenant, KEY, (at(30), at(48)))
        .await
        .unwrap();
    assert_eq!(none, 0, "bug: a window after the night deleted it");

    let spanned = repos
        .sleep
        .delete_sleep_sessions_overlapping(who, &tenant, KEY, (at(0), at(48)))
        .await
        .unwrap();
    assert_eq!(spanned, 1, "bug: a night inside the window survived it");
    assert!(
        night_starts(repos, &tenant, who).await.is_empty(),
        "bug: the night is still read"
    );
}

fn report(record_id: &str) -> SelfReportWrite {
    SelfReportWrite {
        record_id: record_id.to_owned(),
        kind: "mood".to_owned(),
        start_at: at(9),
        end_at: at(9),
        value_code: Some("pleasant".to_owned()),
        value: Some(0.5),
        unit: Some("valence".to_owned()),
        cycle_start: false,
        protection_used: None,
    }
}

async fn file_reports(
    repos: &RepositoryRegistry,
    (tenant, user, provider): (&TenantId, Uuid, &str),
    record_ids: &[&str],
) {
    let source = data_source(repos, tenant, user, provider).await;
    let reports: Vec<SelfReportWrite> = record_ids.iter().map(|id| report(id)).collect();
    repos
        .health_records
        .upsert_self_reports(
            RecordFiling {
                tenant_id: tenant,
                user_id: user,
                provider,
                data_source_id: &source,
            },
            &reports,
        )
        .await
        .unwrap();
}

/// The provider key of every self-report `user` holds in `tenant`, sorted.
async fn reports_of(repos: &RepositoryRegistry, tenant: &TenantId, user: Uuid) -> Vec<String> {
    let mut providers: Vec<String> = repos
        .health_records
        .list_self_reports(
            tenant,
            user,
            RecordWindow {
                since: None,
                until: None,
                limit: 200,
            },
        )
        .await
        .unwrap()
        .into_iter()
        .map(|stored| stored.provider)
        .collect();
    providers.sort();
    providers
}

#[tokio::test]
async fn self_reports_are_deleted_by_record_id_under_their_owner_and_key_only() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let other_tenant = TenantId::parse_str(OTHER_TENANT).unwrap();
    let (who, neighbour) = (user(repos).await, user(repos).await);
    file_reports(repos, (&tenant, who, KEY), &["m1", "m2", "m3"]).await;
    file_reports(repos, (&tenant, who, OTHER_KEY), &["m1"]).await;
    file_reports(repos, (&tenant, neighbour, KEY), &["m1"]).await;
    file_reports(repos, (&other_tenant, who, KEY), &["m1"]).await;

    let deleted = repos
        .health_records
        .delete_self_reports(
            &tenant,
            who,
            KEY,
            &["m1".to_owned(), "m3".to_owned(), "never-stored".to_owned()],
        )
        .await
        .unwrap();

    assert_eq!(deleted, 2, "bug: the two stored ids under the key");
    assert_eq!(
        reports_of(repos, &tenant, who).await,
        vec![OTHER_KEY.to_owned(), KEY.to_owned()],
        "bug: m2 under the key and m1 under another key must stay"
    );
    assert_eq!(
        reports_of(repos, &tenant, neighbour).await,
        vec![KEY.to_owned()],
        "bug: another user's self-report was deleted"
    );
    assert_eq!(
        reports_of(repos, &other_tenant, who).await,
        vec![KEY.to_owned()],
        "bug: the user's self-report in another tenant was deleted"
    );
    assert_eq!(
        repos
            .health_records
            .delete_self_reports(&tenant, who, KEY, &[])
            .await
            .unwrap(),
        0,
        "bug: an empty id list deleted something"
    );
}

fn date(day: u32) -> CivilDate {
    CivilDate::from_ymd(2026, 11, day).unwrap()
}

/// Store a recovery day and a body day dated `day` for `user` under
/// `provider` in `tenant`.
async fn days(
    repos: &RepositoryRegistry,
    (tenant, user, provider): (&TenantId, Uuid, &str),
    day: u32,
) {
    let source = data_source(repos, tenant, user, provider).await;
    let recovery = StoredRecoveryMetrics {
        id: String::new(),
        user_id: user.to_string(),
        data_source_id: source.clone(),
        date: date(day),
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: Some(50.0),
        resting_heart_rate: Some(48),
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: provider.to_owned(),
        recorded_at: at(0),
    };
    repos
        .recovery
        .upsert_recovery_metrics(tenant, &recovery, WellnessWriteMode::FullDay, at(0))
        .await
        .unwrap();
    let body = StoredHealthMetrics {
        id: String::new(),
        user_id: user.to_string(),
        data_source_id: source,
        date: date(day),
        weight_kg: Some(61.0),
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: None,
        vo2_max: None,
        source_name: provider.to_owned(),
        recorded_at: at(0),
    };
    repos
        .health_snapshots
        .upsert_health_snapshot(tenant, &body, WellnessWriteMode::FullDay, at(0))
        .await
        .unwrap();
}

/// The `(provider, date)` of every recovery and body day `user` holds in
/// `tenant`, sorted.
async fn days_held(
    repos: &RepositoryRegistry,
    tenant: &TenantId,
    user: Uuid,
) -> (Vec<(String, CivilDate)>, Vec<(String, CivilDate)>) {
    let (from, to) = (at(-24 * 10), at(24 * 10));
    let mut recovery: Vec<_> = repos
        .recovery
        .get_recovery_metrics(user, tenant, from, to)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.source_name, row.date))
        .collect();
    let mut body: Vec<_> = repos
        .health_snapshots
        .get_health_snapshots(user, tenant, from, to)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.source_name, row.date))
        .collect();
    recovery.sort();
    body.sort();
    (recovery, body)
}

#[tokio::test]
async fn a_dated_delete_removes_only_its_owners_days_of_its_key_in_the_span() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let other_tenant = TenantId::parse_str(OTHER_TENANT).unwrap();
    let (owner, other) = (user(repos).await, user(repos).await);
    for day in [18, 19, 20, 21] {
        days(repos, (&tenant, owner, KEY), day).await;
    }
    days(repos, (&tenant, owner, OTHER_KEY), 19).await;
    days(repos, (&tenant, other, KEY), 19).await;
    days(repos, (&other_tenant, owner, KEY), 19).await;

    let recovery = repos
        .recovery
        .delete_recovery_metrics_dated(owner, &tenant, KEY, (date(19), date(20)))
        .await
        .unwrap();
    let body = repos
        .health_snapshots
        .delete_health_snapshots_dated(owner, &tenant, KEY, (date(19), date(20)))
        .await
        .unwrap();
    assert_eq!(
        (recovery, body),
        (2, 2),
        "bug: both ends of the span are included"
    );

    let kept = vec![
        (KEY.to_owned(), date(18)),
        (OTHER_KEY.to_owned(), date(19)),
        (KEY.to_owned(), date(21)),
    ];
    let mut sorted = kept.clone();
    sorted.sort();
    assert_eq!(
        days_held(repos, &tenant, owner).await,
        (sorted.clone(), sorted),
        "bug: a day outside the span, or of another key, was deleted"
    );
    let theirs = vec![(KEY.to_owned(), date(19))];
    assert_eq!(
        days_held(repos, &tenant, other).await,
        (theirs.clone(), theirs.clone()),
        "bug: another user's day was deleted"
    );
    assert_eq!(
        days_held(repos, &other_tenant, owner).await,
        (theirs.clone(), theirs),
        "bug: the owner's day in another tenant was deleted"
    );
}
