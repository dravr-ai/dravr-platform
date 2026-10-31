// ABOUTME: Stores what one Terra webhook event carried: activities through the activity cache, wellness through the store
// ABOUTME: The records are filed for the athlete the Terra connection belongs to, under the provider key `terra`

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Storing a Terra event
//!
//! [`TerraWebhookHandler`](pierre_providers::terra::TerraWebhookHandler)
//! parses a webhook payload into [`TerraRecords`]; this writes them where
//! every other ingress writes, and nowhere else (Athlete Data Consolidation,
//! S10):
//!
//! - activities through the one `cached_activities` writer, under the key
//!   `terra`, synced at the event's receipt, which `TerraProvider` (a
//!   push-only provider) reads back;
//! - nights, recovery days, body days and daily activity totals through the
//!   [`WellnessStore`], filed under the data source of the service Terra
//!   relayed them from, so the precedence rank reads them as that service's,
//!   relayed.
//!
//! A sleep summary and a daily summary both bring recovery readings for one
//! date, and a body summary may arrive once per measurement, so recovery and
//! body days are partial summaries; a daily summary's activity totals are
//! the whole day's.
//!
//! The caller names the owner: Terra's payload carries Terra's user id, and
//! the route that will receive Terra's webhooks resolves the connection.

use chrono::{DateTime, Utc};
use pierre_core::constants::oauth_providers::TERRA;
use pierre_core::errors::AppResult;
use pierre_core::models::{DataSource, DeviceType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::terra::TerraRecords;
use pierre_services::wellness_store::{WellnessLanded, WellnessStore, WellnessWriteMode};
use uuid::Uuid;

/// What one event's records landed as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerraStored {
    /// Activity rows written (a copy a later receipt already holds is not).
    pub activities: u64,
    /// Nights written (a night a later receipt already holds is not).
    pub nights: usize,
    /// Recovery days written.
    pub recovery: usize,
    /// Body days written.
    pub body: usize,
    /// Daily activity totals written.
    pub daily_activity: usize,
}

/// Store `records`, received at `received_at`, for the athlete `user_id` in
/// `tenant_id`. `relayed_from` is the service Terra relayed them from, as
/// the payload names it (`GARMIN`, `OURA`, ...).
///
/// # Errors
///
/// The first repository error; what was written before it stays.
pub async fn store_terra_records(
    repos: &RepositoryRegistry,
    (tenant_id, user_id): (TenantId, Uuid),
    relayed_from: Option<&str>,
    records: &TerraRecords,
    received_at: DateTime<Utc>,
) -> AppResult<TerraStored> {
    let mut stored = TerraStored::default();
    if !records.activities.is_empty() {
        stored.activities = repos
            .activity_cache
            .upsert_activities_synced_at(
                user_id,
                &tenant_id,
                TERRA,
                &records.activities,
                received_at,
            )
            .await?;
    }
    if records.nights.is_empty()
        && records.recovery.is_empty()
        && records.body.is_empty()
        && records.daily_activity.is_empty()
    {
        return Ok(stored);
    }

    let user = user_id.to_string();
    let store = WellnessStore::new(repos);
    let data_source_id = store
        .record_data_source(
            &tenant_id,
            &DataSource {
                id: Uuid::new_v4().to_string(),
                user_id: user.clone(),
                provider: TERRA.to_owned(),
                device_model: None,
                software_version: None,
                // The relayed service is part of the data source's identity,
                // so a user's WHOOP nights and Garmin nights keep their own
                // source rather than taking the last relay's name.
                source: relayed_from.map(str::to_lowercase),
                device_type: DeviceType::Unknown,
                original_source_name: relayed_from.map(str::to_lowercase),
            },
        )
        .await?;
    let filing = Filing {
        user: &user,
        data_source_id: &data_source_id,
    };

    let mut writes = store.writes("terra webhook");
    for night in &records.nights {
        let mut night = night.clone();
        filing.stamp(
            &mut night.user_id,
            &mut night.data_source_id,
            &mut night.source_name,
        );
        let landed = writes.sleep(tenant_id, &night, received_at).await?;
        stored.nights += usize::from(landed == WellnessLanded::Written);
    }
    for day in &records.recovery {
        let mut day = day.clone();
        filing.stamp(
            &mut day.user_id,
            &mut day.data_source_id,
            &mut day.source_name,
        );
        let landed = writes
            .recovery(
                tenant_id,
                &day,
                WellnessWriteMode::PartialSummary,
                received_at,
            )
            .await?;
        stored.recovery += usize::from(landed == WellnessLanded::Written);
    }
    for day in &records.body {
        let mut day = day.clone();
        filing.stamp(
            &mut day.user_id,
            &mut day.data_source_id,
            &mut day.source_name,
        );
        let landed = writes
            .body(
                tenant_id,
                &day,
                WellnessWriteMode::PartialSummary,
                received_at,
            )
            .await?;
        stored.body += usize::from(landed == WellnessLanded::Written);
    }
    for day in &records.daily_activity {
        let mut day = day.clone();
        filing.stamp(
            &mut day.user_id,
            &mut day.data_source_id,
            &mut day.source_name,
        );
        let landed = writes
            .daily_activity(tenant_id, &day, WellnessWriteMode::FullDay, received_at)
            .await?;
        stored.daily_activity += usize::from(landed == WellnessLanded::Written);
    }
    writes.report();
    Ok(stored)
}

/// Who and which data source a record is filed under.
struct Filing<'a> {
    user: &'a str,
    data_source_id: &'a str,
}

impl Filing<'_> {
    /// Stamp a record's owner, data source and the Terra key.
    fn stamp(&self, user_id: &mut String, data_source_id: &mut String, source_name: &mut String) {
        self.user.clone_into(user_id);
        self.data_source_id.clone_into(data_source_id);
        TERRA.clone_into(source_name);
    }
}
