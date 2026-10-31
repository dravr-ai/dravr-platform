// ABOUTME: The one writer of athlete wellness rows: nights, a day's recovery, body and activity rows, data sources, intraday series
// ABOUTME: Provider syncs, Garmin pushes and archive imports all write through it; WHOOP's terms are applied here, once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The wellness store.
//!
//! Where a day's wellness was stored used to depend on how it arrived: the
//! health sync wrote whole rows through one set of statements, the Garmin
//! push merged them reading by reading through another, on the same keys,
//! each with its own rule for a missing reading (Athlete Data Consolidation
//! design, D2; carnet#1305). Every ingress now writes through this store,
//! and only through it — guard G1 (`scripts/ci/check-one-writer.sh`) fails
//! a repository wellness write called from anywhere else.
//!
//! What differs between ingresses is said, not spelled again: each write
//! names its [`WellnessWriteMode`]. A sync of a whole day writes
//! [`WellnessWriteMode::FullDay`], so a reading the provider no longer holds
//! is cleared; a push of one of the summaries a day arrives in writes
//! [`WellnessWriteMode::PartialSummary`], so the readings other summaries
//! brought stay. A night is always written whole, and only when it was
//! received no earlier than the stored one.
//!
//! WHOOP's terms are applied at this one door, whatever brought the record:
//!
//! - no WHOOP record is kept for an account that owes WHOOP's owner
//!   authorization ([`whoop_terms::owner_authorization_outstanding`]),
//!   decided once per account per run and logged with what was withheld;
//! - WHOOP's own scores are stripped before the write
//!   ([`whoop_terms::sleep_session_to_store`],
//!   [`whoop_terms::recovery_metrics_to_store`]);
//! - and, since a strip only governs what this write binds, every WHOOP
//!   score the account's stored rows still hold is cleared outright after
//!   the write ([`RecoveryRepository::clear_provider_recovery_scores`],
//!   [`SleepRepository::clear_provider_sleep_scores`]), so no write mode, and
//!   no row a write did not touch, can keep one.
//!
//! [`RecoveryRepository::clear_provider_recovery_scores`]: pierre_database::repositories::RecoveryRepository::clear_provider_recovery_scores
//! [`SleepRepository::clear_provider_sleep_scores`]: pierre_database::repositories::SleepRepository::clear_provider_sleep_scores

use std::collections::{HashMap, HashSet};
use std::fmt;

use chrono::{DateTime, Utc};
use dravr_riviere::{DataPoint, SeriesType};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DataSource, StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, TenantId,
};
pub use pierre_database::repositories::WellnessWriteMode;
use pierre_database::RepositoryRegistry;
use tracing::info;
use uuid::Uuid;

use crate::whoop_terms;

/// The series kinds that are intraday samples.
///
/// Heart rate and the two HRV measures, the series the continuous-series
/// read serves. Every other riviere kind is a day's reading (resting heart
/// rate, steps, weight, ...) and belongs on a day's row.
pub const INTRADAY_SERIES: [SeriesType; 3] = [
    SeriesType::HeartRate,
    SeriesType::HrvRmssd,
    SeriesType::HrvSdnn,
];

/// A run of intraday samples of one series: the only shape the series store
/// takes.
///
/// Built only by [`IntradaySeries::try_new`], which refuses a daily kind, so a
/// day's reading cannot reach a store no daily reader looks at.
#[derive(Debug, Clone)]
pub struct IntradaySeries {
    series: SeriesType,
    points: Vec<DataPoint>,
}

impl IntradaySeries {
    /// `points` of `series`, when `series` is one of [`INTRADAY_SERIES`].
    ///
    /// # Errors
    /// Returns an invalid-input error naming a daily kind.
    pub fn try_new(series: SeriesType, points: Vec<DataPoint>) -> AppResult<Self> {
        if INTRADAY_SERIES.contains(&series) {
            Ok(Self { series, points })
        } else {
            Err(AppError::invalid_input(format!(
                "{} is a day's reading, stored on a day's row, never as a series",
                series.as_str()
            )))
        }
    }

    /// The series by riviere's id, when it is intraday.
    ///
    /// # Errors
    /// Returns an invalid-input error for an unknown id or a daily kind.
    pub fn try_from_id(series_type_id: u32, points: Vec<DataPoint>) -> AppResult<Self> {
        let series = SeriesType::from_id(series_type_id).ok_or_else(|| {
            AppError::invalid_input(format!("no series has the id {series_type_id}"))
        })?;
        Self::try_new(series, points)
    }
}

/// What became of one record handed to the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellnessLanded {
    /// It was written.
    Written,
    /// Its terms forbid keeping it: WHOOP's, for an account that owes the
    /// owner authorization.
    Withheld,
    /// A night received later than this one is stored; that one stands.
    Superseded,
}

/// The one writer of wellness rows, over the repositories it writes.
#[derive(Clone, Copy)]
pub struct WellnessStore<'repos> {
    repos: &'repos RepositoryRegistry,
}

impl<'repos> WellnessStore<'repos> {
    /// The store writing through `repos`.
    #[must_use]
    pub const fn new(repos: &'repos RepositoryRegistry) -> Self {
        Self { repos }
    }

    /// Start a run of record writes for `data_type` (named in the log of
    /// what WHOOP's terms withheld). One run decides each account's WHOOP
    /// authorization once and clears its stored WHOOP scores once.
    #[must_use]
    pub fn writes(&self, data_type: &'static str) -> WellnessWrites<'repos> {
        WellnessWrites {
            repos: self.repos,
            data_type,
            authorization: HashMap::new(),
            withheld: 0,
            cleared: HashSet::new(),
        }
    }

    /// Record `source` under `tenant_id`, or refresh the one already
    /// recorded with its identity; returns the stored source's id.
    ///
    /// # Errors
    /// Returns the repository's error when the write fails.
    pub async fn record_data_source(
        &self,
        tenant_id: &TenantId,
        source: &DataSource,
    ) -> AppResult<String> {
        self.repos
            .data_sources
            .upsert_data_source(tenant_id, source)
            .await
    }

    /// Store `series` under the data source `source_id`.
    ///
    /// # Errors
    /// Returns a database error when the series store refuses the batch.
    pub async fn store_series(&self, source_id: &str, series: IntradaySeries) -> AppResult<()> {
        let IntradaySeries { series, points } = series;
        if points.is_empty() {
            return Ok(());
        }
        self.repos
            .time_series_points
            .insert_batch(source_id, series.id(), points)
            .await
            .map_err(|e| AppError::database(format!("store series {}: {e}", series.as_str())))
    }

    /// Delete the night stored under `id`, softly (stamped, read as gone)
    /// or for good. `Ok(false)` when no night has that id; a night already
    /// soft-deleted is found, and a second soft delete leaves its stamp.
    ///
    /// # Errors
    /// Returns the repository's error when the lookup or delete fails.
    pub async fn delete_sleep_session(&self, id: &str, soft: bool) -> AppResult<bool> {
        let Some(tenant_id) = self.repos.sleep.find_sleep_session_tenant(id).await? else {
            return Ok(false);
        };
        self.repos
            .sleep
            .delete_sleep_session_by_id(&tenant_id, id, soft)
            .await?;
        Ok(true)
    }

    /// Delete the recovery day stored under `id`, as
    /// [`Self::delete_sleep_session`] deletes a night.
    ///
    /// # Errors
    /// Returns the repository's error when the lookup or delete fails.
    pub async fn delete_recovery_metric(&self, id: &str, soft: bool) -> AppResult<bool> {
        let Some(tenant_id) = self.repos.recovery.find_recovery_metric_tenant(id).await? else {
            return Ok(false);
        };
        self.repos
            .recovery
            .delete_recovery_metric_by_id(&tenant_id, id, soft)
            .await?;
        Ok(true)
    }

    /// Delete the body day stored under `id`, as
    /// [`Self::delete_sleep_session`] deletes a night.
    ///
    /// # Errors
    /// Returns the repository's error when the lookup or delete fails.
    pub async fn delete_health_snapshot(&self, id: &str, soft: bool) -> AppResult<bool> {
        let Some(tenant_id) = self
            .repos
            .health_snapshots
            .find_health_snapshot_tenant(id)
            .await?
        else {
            return Ok(false);
        };
        self.repos
            .health_snapshots
            .delete_health_snapshot_by_id(&tenant_id, id, soft)
            .await?;
        Ok(true)
    }
}

impl fmt::Debug for WellnessStore<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WellnessStore").finish_non_exhaustive()
    }
}

/// One run of record writes through the [`WellnessStore`].
pub struct WellnessWrites<'repos> {
    repos: &'repos RepositoryRegistry,
    data_type: &'static str,
    /// Per (tenant, user): whether the account still owes WHOOP's owner
    /// authorization, read once per run.
    authorization: HashMap<(TenantId, String), bool>,
    /// How many WHOOP records this run withheld.
    withheld: u64,
    /// The (tenant, user, provider) whose stored scores this run cleared.
    cleared: HashSet<(TenantId, String, String)>,
}

impl fmt::Debug for WellnessWrites<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WellnessWrites")
            .field("data_type", &self.data_type)
            .field("withheld", &self.withheld)
            .finish_non_exhaustive()
    }
}

impl WellnessWrites<'_> {
    /// Write a night received at `synced_at`.
    ///
    /// # Errors
    /// Returns the repository's error when a read or write fails.
    pub async fn sleep(
        &mut self,
        tenant_id: TenantId,
        session: &StoredSleepSession,
        synced_at: DateTime<Utc>,
    ) -> AppResult<WellnessLanded> {
        if self
            .withholds(tenant_id, &session.user_id, &session.source_name)
            .await?
        {
            return Ok(WellnessLanded::Withheld);
        }
        let session = whoop_terms::sleep_session_to_store(session);
        let stored = self
            .repos
            .sleep
            .upsert_sleep_session(&tenant_id, &session, synced_at)
            .await?;
        self.clear_withheld_scores(tenant_id, &session.user_id, &session.source_name)
            .await?;
        Ok(if stored.is_some() {
            WellnessLanded::Written
        } else {
            WellnessLanded::Superseded
        })
    }

    /// Write a day's recovery readings received at `synced_at`, as `mode`
    /// says.
    ///
    /// # Errors
    /// Returns the repository's error when a read or write fails.
    pub async fn recovery(
        &mut self,
        tenant_id: TenantId,
        metrics: &StoredRecoveryMetrics,
        mode: WellnessWriteMode,
        synced_at: DateTime<Utc>,
    ) -> AppResult<WellnessLanded> {
        if self
            .withholds(tenant_id, &metrics.user_id, &metrics.source_name)
            .await?
        {
            return Ok(WellnessLanded::Withheld);
        }
        let metrics = whoop_terms::recovery_metrics_to_store(metrics);
        self.repos
            .recovery
            .upsert_recovery_metrics(&tenant_id, &metrics, mode, synced_at)
            .await?;
        self.clear_withheld_scores(tenant_id, &metrics.user_id, &metrics.source_name)
            .await?;
        Ok(WellnessLanded::Written)
    }

    /// Write a day's body measurements received at `synced_at`, as `mode`
    /// says.
    ///
    /// # Errors
    /// Returns the repository's error when a read or write fails.
    pub async fn body(
        &mut self,
        tenant_id: TenantId,
        snapshot: &StoredHealthMetrics,
        mode: WellnessWriteMode,
        synced_at: DateTime<Utc>,
    ) -> AppResult<WellnessLanded> {
        if self
            .withholds(tenant_id, &snapshot.user_id, &snapshot.source_name)
            .await?
        {
            return Ok(WellnessLanded::Withheld);
        }
        self.repos
            .health_snapshots
            .upsert_health_snapshot(&tenant_id, snapshot, mode, synced_at)
            .await?;
        self.clear_withheld_scores(tenant_id, &snapshot.user_id, &snapshot.source_name)
            .await?;
        Ok(WellnessLanded::Written)
    }

    /// Write a day's activity totals received at `synced_at`, as `mode`
    /// says.
    ///
    /// # Errors
    /// Returns the repository's error when a read or write fails.
    pub async fn daily_activity(
        &mut self,
        tenant_id: TenantId,
        day: &StoredDailyActivity,
        mode: WellnessWriteMode,
        synced_at: DateTime<Utc>,
    ) -> AppResult<WellnessLanded> {
        if self
            .withholds(tenant_id, &day.user_id, &day.source_name)
            .await?
        {
            return Ok(WellnessLanded::Withheld);
        }
        self.repos
            .daily_activity
            .upsert_daily_activity(&tenant_id, day, mode, synced_at)
            .await?;
        Ok(WellnessLanded::Written)
    }

    /// Log what this run withheld, per account it withheld for.
    pub fn report(&self) {
        if self.withheld == 0 {
            return;
        }
        for ((tenant_id, user_id), outstanding) in &self.authorization {
            if *outstanding {
                info!(
                    %tenant_id,
                    user_id = %user_id,
                    data_type = self.data_type,
                    withheld_in_call = self.withheld,
                    "WHOOP records not persisted: the account has not accepted the current WHOOP owner-authorization notice"
                );
            }
        }
    }

    /// Whether a record from `source_name` for `user_id` must not be kept:
    /// it is WHOOP's and the account owes the owner authorization in
    /// `tenant_id`, the tenant it would be written under. Counts what it
    /// withholds.
    async fn withholds(
        &mut self,
        tenant_id: TenantId,
        user_id: &str,
        source_name: &str,
    ) -> AppResult<bool> {
        if !whoop_terms::is_whoop(source_name) {
            return Ok(false);
        }
        let key = (tenant_id, user_id.to_owned());
        let outstanding = if let Some(decided) = self.authorization.get(&key) {
            *decided
        } else {
            let outstanding = whoop_terms::owner_authorization_outstanding(
                self.repos,
                tenant_id.as_uuid(),
                user_uuid(user_id)?,
            )
            .await?;
            self.authorization.insert(key, outstanding);
            outstanding
        };
        if outstanding {
            self.withheld += 1;
        }
        Ok(outstanding)
    }

    /// Clear every score `source_name`'s terms forbid keeping from the rows
    /// `user_id` holds under it, once per run: the strip before a write
    /// governs only what that write binds.
    async fn clear_withheld_scores(
        &mut self,
        tenant_id: TenantId,
        user_id: &str,
        source_name: &str,
    ) -> AppResult<()> {
        if !whoop_terms::scores_withheld(source_name) {
            return Ok(());
        }
        let key = (tenant_id, user_id.to_owned(), source_name.to_owned());
        if self.cleared.contains(&key) {
            return Ok(());
        }
        let user = user_uuid(user_id)?;
        self.repos
            .recovery
            .clear_provider_recovery_scores(user, &tenant_id, source_name)
            .await?;
        self.repos
            .sleep
            .clear_provider_sleep_scores(user, &tenant_id, source_name)
            .await?;
        self.cleared.insert(key);
        Ok(())
    }
}

/// A record's user id as a uuid.
fn user_uuid(user_id: &str) -> AppResult<Uuid> {
    user_id
        .parse::<Uuid>()
        .map_err(|e| AppError::invalid_input(format!("Invalid user_id UUID '{user_id}': {e}")))
}

#[cfg(test)]
mod tests {
    use dravr_riviere::{DataPoint, SeriesType, ALL_SERIES_TYPES};

    use super::{IntradaySeries, INTRADAY_SERIES};

    /// Guard G2: only intraday samples reach the series store. Every other
    /// riviere kind is a day's reading, which belongs on a day's row.
    #[test]
    fn every_daily_kind_is_refused_as_a_series() {
        for series in ALL_SERIES_TYPES {
            let built =
                IntradaySeries::try_new(*series, vec![DataPoint::new(chrono::Utc::now(), 1.0)]);
            assert_eq!(
                built.is_ok(),
                INTRADAY_SERIES.contains(series),
                "bug: {} must be {} as a series",
                series.as_str(),
                if INTRADAY_SERIES.contains(series) {
                    "accepted"
                } else {
                    "refused"
                }
            );
        }
        for daily in [
            SeriesType::RestingHeartRate,
            SeriesType::Steps,
            SeriesType::Weight,
            SeriesType::Vo2Max,
            SeriesType::BodyBattery,
        ] {
            assert!(
                IntradaySeries::try_from_id(daily.id(), Vec::new()).is_err(),
                "bug: the daily kind {} went to the series store by its id",
                daily.as_str()
            );
        }
        assert!(
            IntradaySeries::try_from_id(u32::MAX, Vec::new()).is_err(),
            "an unknown series id is refused"
        );
        assert!(
            IntradaySeries::try_from_id(SeriesType::HeartRate.id(), Vec::new()).is_ok(),
            "bug: intraday heart rate is a series, by its id too"
        );
    }
}
