// ABOUTME: get_heart_rate_series reads back the continuous series a WHOOP sync stored through riviere's TimeSeriesStore
// ABOUTME: Exact points and values, windowed aggregates, metric separation, bounds, and isolation across users and tenants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_heart_rate_series` (carnet#999).
//!
//! Every series is written the way production writes it: a WHOOP
//! `ContinuousMetricBatch` handed to the sync adapter's
//! `store_continuous_metrics`, which lands in `data_point_series` through
//! riviere's `TimeSeriesStore::insert_batch`. The tool is then driven through
//! the `UniversalToolExecutor`, so the read covers the dispatch chokepoint,
//! the tenant scope and riviere's `query`/`aggregate` together.

// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// rustc would demand a crate doc the cfg'd-out file no longer carries.
#![cfg(all(feature = "health-sync", feature = "provider-whoop"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, TimeZone, Utc};
use dravr_enforme::traits::timeseries_store::TimeSeriesPointStore;
use dravr_equilibre_sync::ContinuousMetricBatch;
use dravr_riviere::SeriesType;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::models::{ConnectionType, DataSource, DeviceType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_providers::ai_scope::serve_over;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_tool_runtime::protocols::{ProtocolError, UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

mod common;

const TOOL: &str = "get_heart_rate_series";

/// One sample a minute for an hour, from 06:00.
const SAMPLES: usize = 60;

fn morning() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 6, 0, 0).unwrap()
}

/// `i` minutes, as a duration.
fn minute(i: usize) -> Duration {
    Duration::minutes(i64::try_from(i).unwrap())
}

/// Sample `i`'s heart rate: a climb from 50 bpm, one beat a minute.
fn bpm(i: usize) -> f64 {
    50.0 + i as f64
}

async fn executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

/// A user, the tenant they own and a WHOOP connection in it, without having
/// accepted WHOOP's notice.
async fn unconsented_whoop_athlete(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    let email = format!("hr-series-{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let repos = executor.resources.repos();
    let tenant = repos
        .tenants
        .get_all()
        .await?
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should own a tenant"))?
        .id;
    repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await?;
    Ok((user_id, tenant))
}

/// [`unconsented_whoop_athlete`] who accepted WHOOP's notice: the owner
/// authorization a WHOOP sync runs under, and the consent to hand its data
/// to a model (carnet#726).
async fn whoop_athlete(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    let athlete = unconsented_whoop_athlete(executor).await?;
    executor
        .resources
        .repos()
        .users
        .record_provider_terms(
            athlete.0,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await?;
    Ok(athlete)
}

/// Sync one WHOOP batch of `series` for the athlete through the adapter
/// production writes with; returns the data source it landed under.
async fn sync_whoop_batch(
    executor: &UniversalToolExecutor,
    (user_id, tenant): (Uuid, TenantId),
    series: SeriesType,
    points: Vec<(DateTime<Utc>, f64)>,
) -> Result<String> {
    let repos = executor.resources.repos();
    let source_id = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: Some("whoop".to_owned()),
            },
        )
        .await?;
    let expected = points.len() as u64;
    let written = PierreSyncStorage::new(repos)
        .store_continuous_metrics(
            &source_id,
            &[ContinuousMetricBatch {
                series_type_id: series.id(),
                points,
            }],
        )
        .await?;
    assert_eq!(written, expected);
    Ok(source_id)
}

/// The morning's hour of heart rate, one sample a minute.
fn morning_hour() -> Vec<(DateTime<Utc>, f64)> {
    (0..SAMPLES)
        .map(|i| (morning() + minute(i), bpm(i)))
        .collect()
}

/// Call the tool over `transport`.
async fn call_over(
    transport: Transport,
    executor: &UniversalToolExecutor,
    (user_id, tenant): (Uuid, TenantId),
    params: Value,
) -> Result<Value, ProtocolError> {
    let response = serve_over(
        transport,
        executor.execute_tool(UniversalRequest {
            tool_name: TOOL.to_owned(),
            parameters: params,
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        }),
    )
    .await?;
    assert!(response.success, "{TOOL} failed: {:?}", response.error);
    Ok(response.result.unwrap())
}

/// Call the tool from the athlete's own web app: WHOOP keeps its data to
/// Dravr's surfaces (carnet#766), so this is where its series is served.
async fn call(
    executor: &UniversalToolExecutor,
    athlete: (Uuid, TenantId),
    params: Value,
) -> Result<Value, ProtocolError> {
    call_over(Transport::WebApp, executor, athlete, params).await
}

fn morning_range() -> Value {
    json!({
        "start": morning().to_rfc3339(),
        "end": (morning() + Duration::hours(1)).to_rfc3339(),
    })
}

fn with(mut base: Value, extra: &Value) -> Value {
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

#[tokio::test]
async fn tool_is_registered() -> Result<()> {
    let executor = executor().await?;
    assert!(executor
        .resources
        .tool_registry()
        .tool_names()
        .contains(&TOOL));
    Ok(())
}

#[tokio::test]
async fn reads_back_exactly_the_points_a_whoop_sync_stored() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    let source_id =
        sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;

    let result = call(&executor, athlete, morning_range()).await?;

    assert_eq!(result["metric"], "heart_rate");
    assert_eq!(result["unit"], "bpm");
    assert!(result.get("window").is_none());
    assert_eq!(result["count"], SAMPLES);
    let sources = result["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["data_source_id"], source_id.as_str());
    assert_eq!(sources[0]["provider"], "whoop");
    let points = sources[0]["points"].as_array().unwrap();
    assert_eq!(points.len(), SAMPLES);
    for (i, point) in points.iter().enumerate() {
        let at = DateTime::parse_from_rfc3339(point["timestamp"].as_str().unwrap())?;
        assert_eq!(at, morning() + minute(i));
        assert_eq!(point["value"].as_f64().unwrap(), bpm(i));
    }
    Ok(())
}

#[tokio::test]
async fn the_range_is_half_open_and_echoed() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;

    // [06:10, 06:20) holds samples 10..=19 and not the one at 06:20.
    let start = morning() + Duration::minutes(10);
    let end = morning() + Duration::minutes(20);
    let result = call(
        &executor,
        athlete,
        json!({ "start": start.to_rfc3339(), "end": end.to_rfc3339() }),
    )
    .await?;

    assert_eq!(result["count"], 10);
    let points = result["sources"][0]["points"].as_array().unwrap();
    assert_eq!(points.first().unwrap()["value"].as_f64().unwrap(), bpm(10));
    assert_eq!(points.last().unwrap()["value"].as_f64().unwrap(), bpm(19));
    assert_eq!(result["range"]["start"], start.to_rfc3339());
    assert_eq!(result["range"]["end"], end.to_rfc3339());
    Ok(())
}

#[tokio::test]
async fn aggregates_the_series_into_windows() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;

    // Four 15-minute windows; window w holds samples 15w..15w+14.
    let avg = call(
        &executor,
        athlete,
        with(morning_range(), &json!({ "window_minutes": 15 })),
    )
    .await?;
    assert_eq!(avg["window"]["minutes"], 15);
    assert_eq!(avg["window"]["aggregation"], "avg");
    assert_eq!(avg["count"], 4);
    let windows = avg["sources"][0]["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 4);
    for (w, window) in windows.iter().enumerate() {
        let first = 15 * w;
        let mean = (first..first + 15).map(bpm).sum::<f64>() / 15.0;
        assert_eq!(window["value"].as_f64().unwrap(), mean);
        assert_eq!(window["sample_count"], 15);
        let opens = DateTime::parse_from_rfc3339(window["window_start"].as_str().unwrap())?;
        assert_eq!(opens, morning() + minute(first));
    }
    assert!(avg["sources"][0].get("points").is_none());

    let max = call(
        &executor,
        athlete,
        with(
            morning_range(),
            &json!({ "window_minutes": 30, "aggregation": "max" }),
        ),
    )
    .await?;
    let windows = max["sources"][0]["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0]["value"].as_f64().unwrap(), bpm(29));
    assert_eq!(windows[1]["value"].as_f64().unwrap(), bpm(59));

    // A window wider than the hour is one window over all of it.
    let whole = call(
        &executor,
        athlete,
        with(
            morning_range(),
            &json!({ "window_minutes": 120, "aggregation": "min" }),
        ),
    )
    .await?;
    let windows = whole["sources"][0]["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0]["sample_count"], SAMPLES);
    assert_eq!(windows[0]["value"].as_f64().unwrap(), bpm(0));
    Ok(())
}

#[tokio::test]
async fn each_metric_reads_only_its_own_series() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;
    let rmssd = vec![(morning() + Duration::minutes(5), 62.5)];
    sync_whoop_batch(&executor, athlete, SeriesType::HrvRmssd, rmssd).await?;

    let hrv = call(
        &executor,
        athlete,
        with(morning_range(), &json!({ "metric": "hrv_rmssd" })),
    )
    .await?;
    assert_eq!(hrv["metric"], "hrv_rmssd");
    assert_eq!(hrv["unit"], "ms");
    assert_eq!(hrv["count"], 1);
    assert_eq!(
        hrv["sources"][0]["points"][0]["value"].as_f64().unwrap(),
        62.5
    );

    let sdnn = call(
        &executor,
        athlete,
        with(morning_range(), &json!({ "metric": "hrv_sdnn" })),
    )
    .await?;
    assert_eq!(sdnn["count"], 0);
    assert_eq!(
        sdnn["sources"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    Ok(())
}

#[tokio::test]
async fn an_athlete_reads_only_their_own_series() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    let other = whoop_athlete(&executor).await?;
    let mine = sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;
    let theirs = sync_whoop_batch(
        &executor,
        other,
        SeriesType::HeartRate,
        vec![(morning() + Duration::minutes(30), 180.0)],
    )
    .await?;
    assert_ne!(mine, theirs);

    // Each reads their own series, never the other's.
    let result = call(&executor, athlete, morning_range()).await?;
    assert_eq!(result["count"], SAMPLES);
    let sources = result["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["data_source_id"], mine.as_str());
    assert!(sources[0]["points"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["value"].as_f64().unwrap() < 180.0));

    let result = call(&executor, other, morning_range()).await?;
    assert_eq!(result["count"], 1);
    assert_eq!(result["sources"][0]["data_source_id"], theirs.as_str());

    // A data-source id is not an argument: naming the other athlete's
    // source reads nothing of theirs.
    let result = call(
        &executor,
        athlete,
        with(morning_range(), &json!({ "data_source_id": theirs })),
    )
    .await?;
    assert_eq!(result["count"], SAMPLES);
    assert_eq!(result["sources"][0]["data_source_id"], mine.as_str());
    Ok(())
}

#[tokio::test]
async fn another_tenant_reads_none_of_the_series() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    let other = whoop_athlete(&executor).await?;
    sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;

    // The athlete's own id under the other tenant: their source belongs to
    // their tenant, so the read finds nothing.
    let result = call(&executor, (athlete.0, other.1), morning_range()).await?;
    assert_eq!(result["count"], 0);
    assert_eq!(
        result["sources"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );

    // Under their own tenant the same read finds the whole hour.
    let own = call(&executor, athlete, morning_range()).await?;
    assert_eq!(own["count"], SAMPLES);
    Ok(())
}

#[tokio::test]
async fn a_whoop_series_stays_off_external_transports() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;
    sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;

    // WHOOP's terms keep its data first-party: an MCP client over HTTP reads
    // none of the series the athlete's app reads in full.
    let external = call_over(Transport::McpHttp, &executor, athlete, morning_range()).await?;
    assert_eq!(external["count"], 0);
    assert_eq!(
        external["sources"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );

    let own = call(&executor, athlete, morning_range()).await?;
    assert_eq!(own["count"], SAMPLES);
    Ok(())
}

#[tokio::test]
async fn a_whoop_series_is_withheld_from_the_model_without_consent() -> Result<()> {
    let executor = executor().await?;
    let athlete = unconsented_whoop_athlete(&executor).await?;
    sync_whoop_batch(&executor, athlete, SeriesType::HeartRate, morning_hour()).await?;

    // The athlete never accepted WHOOP's notice, so the series stays out of
    // the model's reach and the answer says data was held back.
    let result = call(&executor, athlete, morning_range()).await?;
    assert_eq!(result["count"], 0);
    assert_eq!(
        result["sources"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    assert_eq!(result["_withheld"]["items_dropped"], 1);
    assert_eq!(result["_withheld"]["sources"], json!(["whoop"]));
    Ok(())
}

#[tokio::test]
async fn bounds_and_vocabulary_are_enforced() -> Result<()> {
    let executor = executor().await?;
    let athlete = whoop_athlete(&executor).await?;

    for params in [
        with(morning_range(), &json!({ "metric": "steps" })),
        with(morning_range(), &json!({ "aggregation": "max" })),
        with(
            morning_range(),
            &json!({ "window_minutes": 5, "aggregation": "sum" }),
        ),
        with(morning_range(), &json!({ "window_minutes": 0 })),
        json!({ "start": "yesterday" }),
        json!({
            "start": (morning() + Duration::hours(1)).to_rfc3339(),
            "end": morning().to_rfc3339(),
        }),
        // A day in one-minute windows is 1440 windows, past the cap.
        json!({
            "start": morning().to_rfc3339(),
            "end": (morning() + Duration::days(1)).to_rfc3339(),
            "window_minutes": 1,
        }),
    ] {
        let refused = call(&executor, athlete, params.clone()).await;
        assert!(
            matches!(
                refused,
                Err(ProtocolError::InvalidParameters(_) | ProtocolError::InvalidRequest(_))
            ),
            "{params} should be refused, got {refused:?}"
        );
    }

    // A sample read wider than 48 hours keeps its most recent 48.
    let end = morning() + Duration::days(5);
    let result = call(
        &executor,
        athlete,
        json!({ "start": morning().to_rfc3339(), "end": end.to_rfc3339() }),
    )
    .await?;
    assert_eq!(
        result["range"]["start"],
        (end - Duration::hours(48)).to_rfc3339()
    );
    Ok(())
}
