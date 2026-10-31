// ABOUTME: Pins the one wellness read: the sleep tools date a night by the athlete's morning, merged once
// ABOUTME: A 23:00-07:00 night at UTC+10 meets its own morning's recovery; a named source narrows the merge

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every sleep and recovery reader reads through
//! `pierre_services::stored_health`'s merged reads (carnet#1281). Before it,
//! the sleep tools paired a night with the recovery of its UTC end date, so
//! an athlete far east of UTC — whose 07:00 wake-up is the previous evening
//! in UTC — had every night paired with the wrong morning: the HRV a WHOOP
//! strap reports on recovery never reached the night it belongs to.

use anyhow::Result;
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Australia::Brisbane;
use pierre_core::models::{
    ConnectionType, DataSource, DeviceType, StoredRecoveryMetrics, StoredSleepSession, TenantId,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use pierre_providers::precedence::{Ingress, Service, Source};
use pierre_services::stored_health;
use pierre_tool_runtime::protocol::sleep_helpers::{latest_sleep_data, stored_sleep_nights};
use pierre_tool_runtime::protocols::UniversalToolExecutor;
use std::sync::Arc;
use uuid::Uuid;

use crate::common;
use pierre_database::repositories::WellnessWriteMode;

/// An executor serving the athlete's own web app.
async fn executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp),
    ))
}

/// An athlete living in Brisbane (UTC+10, no daylight saving), with a tenant.
async fn brisbane_athlete(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    athlete_in(executor, "Australia/Brisbane").await
}

/// An athlete living in `zone`, with a tenant and WHOOP and Garmin connected.
async fn athlete_in(executor: &UniversalToolExecutor, zone: &str) -> Result<(Uuid, TenantId)> {
    let email = format!("chokepoint_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let repos = executor.resources.repos();
    repos.users.set_timezone(user_id, zone).await?;
    let tenant = repos
        .tenants
        .get_all()
        .await?
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?
        .id;
    let tenant = TenantId::parse_str(&tenant.to_string())?;
    for provider in ["whoop", "garmin"] {
        repos
            .provider_connections
            .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
            .await?;
    }
    Ok((user_id, tenant))
}

async fn data_source(
    executor: &UniversalToolExecutor,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
) -> Result<String> {
    Ok(executor
        .resources
        .repos()
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await?)
}

/// Yesterday's local morning in Brisbane: the night ended at 07:00 there.
fn brisbane_morning() -> NaiveDate {
    Utc::now().with_timezone(&Brisbane).date_naive() - Duration::days(1)
}

/// The 23:00-07:00 night ending on `morning` in Brisbane, from `source`.
fn night(user_id: Uuid, source: &str, ds: &str, morning: NaiveDate) -> StoredSleepSession {
    let end = Brisbane
        .from_local_datetime(&morning.and_hms_opt(7, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc);
    StoredSleepSession {
        id: format!("{source}-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: ds.to_owned(),
        is_nap: false,
        start_datetime: end - Duration::hours(8),
        end_datetime: end,
        total_sleep_seconds: Some(7 * 3600),
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
        source_name: source.to_owned(),
    }
}

fn morning_recovery(
    user_id: Uuid,
    source: &str,
    ds: &str,
    date: NaiveDate,
    hrv_rmssd: f64,
) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: format!("{source}-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: ds.to_owned(),
        date,
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: Some(hrv_rmssd),
        resting_heart_rate: Some(46),
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: source.to_owned(),
        recorded_at: Utc::now(),
    }
}

/// T4 (sleep tools): the night ended at 21:00 UTC the evening before the
/// athlete's morning; the tools pair it with that morning's recovery, the
/// date the snapshot and the readiness rail give it too.
#[tokio::test]
async fn a_night_at_utc_plus_ten_meets_its_own_mornings_recovery() -> Result<()> {
    let executor = executor().await?;
    let athlete = brisbane_athlete(&executor).await?;
    let (user_id, tenant) = athlete;
    let garmin = data_source(&executor, athlete, "garmin").await?;
    let repos = executor.resources.repos();
    let morning = brisbane_morning();

    let night = night(user_id, "garmin", &garmin, morning);
    assert_ne!(
        night.end_datetime.date_naive(),
        morning,
        "the fixture's night must end on another UTC date than its local one"
    );
    repos
        .sleep
        .upsert_sleep_session(&tenant, &night, Utc::now())
        .await?;
    // The UTC date of the night's end carries a decoy morning: a reader that
    // dated the night by UTC would pick it up instead.
    for (date, hrv) in [(morning, 63.0), (night.end_datetime.date_naive(), 31.0)] {
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &morning_recovery(user_id, "garmin", &garmin, date, hrv),
                WellnessWriteMode::FullDay,
                Utc::now(),
            )
            .await?;
    }

    let tenant_id = tenant.to_string();
    let (data, sources) = latest_sleep_data(&executor, user_id, Some(&tenant_id), None, 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(sources, vec!["garmin".to_owned()], "one source synced it");
    assert_eq!(
        data.hrv_rmssd_ms,
        Some(63.0),
        "the HRV of the athlete's own morning, not of the UTC date"
    );
    assert_eq!(
        data.resting_hr_bpm,
        Some(46),
        "the morning fills resting HR"
    );
    Ok(())
}

/// One night two sources recorded is one merged night; naming a source
/// narrows the read to that source's rows before the merge.
#[tokio::test]
async fn a_named_sleep_source_narrows_the_merged_night() -> Result<()> {
    let executor = executor().await?;
    let athlete = brisbane_athlete(&executor).await?;
    let (user_id, tenant) = athlete;
    let garmin = data_source(&executor, athlete, "garmin").await?;
    let whoop = data_source(&executor, athlete, "whoop").await?;
    let repos = executor.resources.repos();
    let morning = brisbane_morning();
    let mut garmin_night = night(user_id, "garmin", &garmin, morning);
    garmin_night.deep_sleep_seconds = Some(5_400);
    let mut whoop_night = night(user_id, "whoop", &whoop, morning);
    whoop_night.total_sleep_seconds = Some(6 * 3600);
    repos
        .sleep
        .upsert_sleep_session(&tenant, &garmin_night, Utc::now())
        .await?;
    repos
        .sleep
        .upsert_sleep_session(&tenant, &whoop_night, Utc::now())
        .await?;

    let tenant_id = tenant.to_string();
    let every = stored_sleep_nights(&executor, user_id, Some(&tenant_id), None, 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(every.len(), 1, "one night across both sources");
    let mut sources = every[0].sources.clone();
    sources.sort();
    assert_eq!(
        sources,
        vec!["garmin".to_owned(), "whoop".to_owned()],
        "both sources merge"
    );

    let garmin_only = stored_sleep_nights(&executor, user_id, Some(&tenant_id), Some("Garmin"), 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(garmin_only.len(), 1, "one Garmin night");
    assert_eq!(
        garmin_only[0].sources,
        vec!["garmin".to_owned()],
        "the named source alone"
    );
    assert_eq!(
        garmin_only[0].record.total_sleep_seconds,
        Some(7 * 3600),
        "Garmin's own duration, not WHOOP's"
    );
    assert_eq!(garmin_only[0].filled.len(), 0, "WHOOP took no part");
    Ok(())
}

/// A named sleep source narrows the recovery that fills the night too: a
/// Garmin night never carries WHOOP's HRV or resting HR under Garmin's name.
#[tokio::test]
async fn a_named_sleep_source_takes_no_other_sources_recovery() -> Result<()> {
    let executor = executor().await?;
    let athlete = brisbane_athlete(&executor).await?;
    let (user_id, tenant) = athlete;
    let garmin = data_source(&executor, athlete, "garmin").await?;
    let whoop = data_source(&executor, athlete, "whoop").await?;
    let repos = executor.resources.repos();
    let morning = brisbane_morning();
    repos
        .sleep
        .upsert_sleep_session(
            &tenant,
            &night(user_id, "garmin", &garmin, morning),
            Utc::now(),
        )
        .await?;
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &morning_recovery(user_id, "whoop", &whoop, morning, 70.0),
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;

    let tenant_id = tenant.to_string();
    let (garmin_only, sources) =
        latest_sleep_data(&executor, user_id, Some(&tenant_id), Some("garmin"), 7)
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(sources, vec!["garmin".to_owned()]);
    assert_eq!(garmin_only.hrv_rmssd_ms, None, "WHOOP's HRV stays out");
    assert_eq!(garmin_only.resting_hr_bpm, None, "WHOOP's resting HR too");

    let (every, _) = latest_sleep_data(&executor, user_id, Some(&tenant_id), None, 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(
        every.hrv_rmssd_ms,
        Some(70.0),
        "unnamed, every source fills"
    );
    Ok(())
}

/// A day-summary night (Garmin capture, intervals.icu) has no bedtime and is
/// anchored at midnight UTC of its date. West of UTC a 6 h night "ends" the
/// local evening before; it is still that date's night and meets that
/// morning's recovery, not the previous one.
#[tokio::test]
async fn a_day_summary_night_west_of_utc_meets_its_own_date() -> Result<()> {
    let executor = executor().await?;
    let athlete = athlete_in(&executor, "America/Los_Angeles").await?;
    let (user_id, tenant) = athlete;
    let garmin = data_source(&executor, athlete, "garmin").await?;
    let repos = executor.resources.repos();
    let date = Utc::now().date_naive() - Duration::days(1);
    let mut summary = night(user_id, "garmin", &garmin, date);
    summary.start_datetime = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    summary.end_datetime = summary.start_datetime + Duration::hours(6);
    repos
        .sleep
        .upsert_sleep_session(&tenant, &summary, Utc::now())
        .await?;
    for (day, hrv) in [(date, 58.0), (date - Duration::days(1), 39.0)] {
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &morning_recovery(user_id, "garmin", &garmin, day, hrv),
                WellnessWriteMode::FullDay,
                Utc::now(),
            )
            .await?;
    }

    let tenant_id = tenant.to_string();
    let (latest, _) = latest_sleep_data(&executor, user_id, Some(&tenant_id), None, 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(
        latest.hrv_rmssd_ms,
        Some(58.0),
        "the summary's own morning, not the day before"
    );
    Ok(())
}

// ============================================================================
// One precedence (Athlete Data Consolidation S2, carnet#1303)
// ============================================================================

/// T7: a WHOOP band and a Garmin watch recorded the same night. WHOOP stands
/// for it under the one rank, and the athlete's sleep source — what the
/// agent proposal names — is WHOOP, the merged night's primary, not the
/// alphabetically first connection.
#[tokio::test]
async fn a_whoop_and_a_garmin_night_stand_as_whoops_everywhere() -> Result<()> {
    let executor = executor().await?;
    let athlete = brisbane_athlete(&executor).await?;
    let (user_id, tenant) = athlete;
    let garmin = data_source(&executor, athlete, "garmin").await?;
    let whoop = data_source(&executor, athlete, "whoop").await?;
    let repos = executor.resources.repos();
    let morning = brisbane_morning();
    let mut garmin_night = night(user_id, "garmin", &garmin, morning);
    garmin_night.deep_sleep_seconds = Some(5_400);
    let mut whoop_night = night(user_id, "whoop", &whoop, morning);
    whoop_night.total_sleep_seconds = Some(6 * 3600 + 1_800);
    repos
        .sleep
        .upsert_sleep_session(&tenant, &garmin_night, Utc::now())
        .await?;
    repos
        .sleep
        .upsert_sleep_session(&tenant, &whoop_night, Utc::now())
        .await?;

    let tenant_id = tenant.to_string();
    let nights = stored_sleep_nights(&executor, user_id, Some(&tenant_id), None, 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(
        nights.len(),
        1,
        "bug: the two sources' nights did not merge into one"
    );
    assert_eq!(nights[0].record.source_name, "whoop");
    assert_eq!(
        nights[0].primary.service,
        Service::Whoop,
        "bug: the night's primary is not the best-ranked source"
    );
    assert_eq!(
        nights[0].record.total_sleep_seconds,
        Some(6 * 3600 + 1_800),
        "bug: the night's sleep is not its primary's"
    );
    assert_eq!(
        nights[0].record.deep_sleep_seconds,
        Some(5_400),
        "Garmin fills"
    );

    let now = Utc::now();
    let source = stored_health::latest_night_source(
        repos,
        executor.resources.provider_registry().as_ref(),
        user_id,
        &tenant,
        (now - Duration::days(14), now),
    )
    .await?;
    assert_eq!(source.as_deref(), Some("whoop"));
    Ok(())
}

/// T4b: the same night reaches Dravr as a Garmin capture, anchored at
/// midnight UTC of its date, and as the athlete's Garmin export with the real
/// bedtime (23:00-07:00 in Brisbane, ending 21:00 UTC the evening before).
/// The two windows never overlap, and they are one night: Garmin's own copy
/// first, the export filling what it lacks.
#[tokio::test]
async fn a_capture_and_an_export_of_one_night_far_from_utc_are_one_night() -> Result<()> {
    let executor = executor().await?;
    let athlete = brisbane_athlete(&executor).await?;
    let (user_id, tenant) = athlete;
    let garmin = data_source(&executor, athlete, "garmin").await?;
    let export = data_source(&executor, athlete, "garmin_archive").await?;
    let repos = executor.resources.repos();
    let morning = brisbane_morning();
    let mut capture = night(user_id, "garmin", &garmin, morning);
    capture.start_datetime = morning.and_hms_opt(0, 0, 0).unwrap().and_utc();
    capture.end_datetime = capture.start_datetime + Duration::hours(8);
    let mut exported = night(user_id, "garmin_archive", &export, morning);
    exported.deep_sleep_seconds = Some(4_800);
    assert!(
        exported.end_datetime <= capture.start_datetime,
        "the fixture's two copies must not overlap"
    );
    repos
        .sleep
        .upsert_sleep_session(&tenant, &capture, Utc::now())
        .await?;
    repos
        .sleep
        .upsert_sleep_session(&tenant, &exported, Utc::now())
        .await?;

    let tenant_id = tenant.to_string();
    let nights = stored_sleep_nights(&executor, user_id, Some(&tenant_id), None, 7)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.error))?;
    assert_eq!(nights.len(), 1, "{nights:?}");
    assert_eq!(
        nights[0].record.source_name, "garmin",
        "the service's own copy"
    );
    assert_eq!(
        nights[0].record.deep_sleep_seconds,
        Some(4_800),
        "the export fills"
    );
    Ok(())
}

/// T11 (terms): a Garmin export's day reaches a model like the athlete's
/// other data, under the terms its own key declares, never by the absence of
/// any rule — and it ranks as Garmin's own day under the service's API copy.
#[tokio::test]
async fn an_exported_garmin_day_survives_a_model_read_under_its_own_terms() -> Result<()> {
    let executor = executor().await?;
    let athlete = brisbane_athlete(&executor).await?;
    let (user_id, tenant) = athlete;
    let export = data_source(&executor, athlete, "garmin_archive").await?;
    let repos = executor.resources.repos();
    let registry = executor.resources.provider_registry();
    assert!(
        registry.get_descriptor("garmin_archive").is_some(),
        "the export's key declares its terms"
    );
    let morning = brisbane_morning();
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &morning_recovery(user_id, "garmin_archive", &export, morning, 57.0),
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;

    let now = Utc::now();
    let (read, _) = ai_scope::ai_read(stored_health::merged_recovery(
        repos,
        registry.as_ref(),
        user_id,
        &tenant,
        (now - Duration::days(7), now),
        stored_health::SourceFilter::All,
    ))
    .await;
    let days = read?.records;
    assert_eq!(days.len(), 1, "the export's day reaches the model");
    assert_eq!(
        days[0].record.daily_hrv(),
        Some(57.0),
        "bug: the day's HRV is not its primary's"
    );
    assert_eq!(
        days[0].primary,
        Source::new(Service::Garmin, Ingress::Export)
    );
    Ok(())
}
