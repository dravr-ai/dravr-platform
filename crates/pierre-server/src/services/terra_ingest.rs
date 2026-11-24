// ABOUTME: Stores what one Terra webhook event carried: activities through the activity store, wellness through its store
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
//! - activities through the activity store ([`LiveActivityStore`]), under
//!   the key `terra`, received at the event's receipt, which serves them as
//!   the cached rows `TerraProvider` (a push-only provider) reads back;
//! - nights, recovery days, body days and daily activity totals through the
//!   one wellness projection ([`wellness_projection::project`]), filed under
//!   the data source of the service Terra relayed them from, so the
//!   precedence rank reads them as that service's, relayed.
//!
//! A sleep summary and a daily summary both bring recovery readings for one
//! date, and a body summary may arrive once per measurement, so recovery and
//! body days are partial summaries; a daily summary's activity totals are
//! the whole day's.
//!
//! The caller names the owner: Terra's payload carries Terra's user id, and
//! the route that will receive Terra's webhooks resolves the connection.

use dravr_barrage::WellnessRecords;
use dravr_tronc::dates::Timestamp;
use pierre_core::constants::oauth_providers::TERRA;
use pierre_core::errors::AppResult;
use pierre_core::models::{DataSource, DeviceType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::terra::TerraRecords;
use pierre_services::live_activities::LiveActivityStore;
use pierre_services::wellness_projection::{self, WellnessFiling, WellnessModes};
use pierre_services::wellness_store::WellnessWriteMode;
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
/// `tenant_id`, the activities through `live`.
///
/// `relayed_from` is the service Terra relayed them from, as the payload
/// names it (`GARMIN`, `OURA`, ...).
///
/// # Errors
///
/// The first repository error; what was written before it stays.
pub async fn store_terra_records(
    repos: &RepositoryRegistry,
    live: &dyn LiveActivityStore,
    (tenant_id, user_id): (TenantId, Uuid),
    relayed_from: Option<&str>,
    records: &TerraRecords,
    received_at: Timestamp,
) -> AppResult<TerraStored> {
    let mut stored = TerraStored::default();
    for activity in &records.activities {
        // LIMITATION(registre#1662): store_terra_records drops LiveIngest::landed, so a Terra-relayed workout announces no landing
        if live
            .ingest_received(user_id, tenant_id, TERRA, activity, received_at)
            .await?
            .stored
            > 0
        {
            stored.activities += 1;
        }
    }
    if records.nights.is_empty()
        && records.recovery.is_empty()
        && records.body.is_empty()
        && records.daily_activity.is_empty()
    {
        return Ok(stored);
    }

    let wellness = WellnessRecords {
        sleep: records.nights.clone(),
        recovery: records.recovery.clone(),
        body: records.body.clone(),
        daily_activity: records.daily_activity.clone(),
        ..WellnessRecords::default()
    };
    let filing = WellnessFiling {
        tenant_id,
        user_id,
        provider: TERRA,
        data_source: DataSource {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
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
        modes: WellnessModes {
            recovery: WellnessWriteMode::PartialSummary,
            body: WellnessWriteMode::PartialSummary,
            daily_activity: WellnessWriteMode::FullDay,
        },
        label: "terra webhook",
    };
    let counts = wellness_projection::project(repos, filing, &wellness, received_at).await?;
    stored.nights = counts.nights;
    stored.recovery = counts.recovery;
    stored.body = counts.body;
    stored.daily_activity = counts.daily_activity;
    Ok(stored)
}
