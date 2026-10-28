// ABOUTME: Garmin push ingest: one stored batch's records, each resolved to its owner and written to Dravr's stores
// ABOUTME: Activities to the activity cache, day metrics to the health stores, consent changes to purges and disconnects

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Garmin push ingest
//!
//! The Garmin Connect Developer Program at the Starter tier delivers data
//! only by pushing it (carnet#1155): no list, no detail read and no backfill
//! exist, so nothing here ever asks Garmin for data a push did not bring. The
//! route stores each pushed batch in `garmin_push_inbox` before it answers;
//! [`ingest_batch`] is what a worker runs over one stored batch.
//!
//! Each record names its Garmin user. The user resolves to a platform owner
//! through the stored `garmin` token that captured that id
//! ([`resolve_webhook_owner`]); a record whose user names nobody, belonged
//! to a deleted account, or would be read through a coach's link is dropped,
//! and the log line carries counts, never the record.
//!
//! What a batch writes, by summary type:
//!
//! | Summary | Store |
//! |---|---|
//! | `activities` | `cached_activities` under `garmin`, id = `activityId` |
//! | `activityDetails` | the summary as above, laps beside it, samples in `activity_streams` |
//! | `activityFiles` | samples and laps from the FIT file, for activities over 24 h only |
//! | `dailies` | `recovery_metrics` (resting heart rate, stress), heart-rate series |
//! | `sleeps` | `sleep_sessions` |
//! | `hrv` | `recovery_metrics` (rMSSD), rMSSD series |
//! | `userMetrics` | `health_snapshots` (VO2max) |
//! | `bodyComps` | `health_snapshots` |
//! | `stressDetails` | the day's Body Battery on `recovery_metrics` |
//! | `skinTemp` | `recovery_metrics` (skin-temperature deviation) |
//! | `deregistrations` | the owner's Garmin connection withdrawn |
//! | `userPermissionsChange` | the grants recorded; withdrawn data purged |
//!
//! Intraday `SpO2`, breathing-rate and stress readings (`pulseox`,
//! `allDayRespiration`, and the series inside `sleeps` and `stressDetails`)
//! are not kept: no reader serves them (the continuous-series read covers
//! heart rate and HRV only), so storing them would hold athlete data no
//! one can see. A `pulseox` or `allDayRespiration` push is refused as a type
//! Dravr does not ingest, so neither should be subscribed in the portal.
//!
//! A day's recovery and body rows are fed by several summary types, so each
//! push merges only what it carries into the day's row, in one statement, and
//! two pushes for one day may run at once. Health summaries are refused for an
//! owner whose last recorded grants leave out `HEALTH_EXPORT`, activity
//! summaries without `ACTIVITY_EXPORT`.
//!
//! Every write is an upsert keyed by Garmin's ids or the summary's day, so a
//! batch processed twice, or a record Garmin re-sends as an update, lands
//! the same rows again with the latest values. A batch retried after a later
//! push for the same activity, night or samples landed leaves that later
//! push's row as it is (`PushedDataRepository`).
//!
//! Wellness is filed under the owner's Garmin API data source, marked
//! `API_SOURCE_MARKER`, apart from the source the usr/pwd connection's
//! health sync files its `garmin` rows under, so withdrawing the API deletes
//! what the API wrote and leaves that connection's history.
//!
//! A batch's owner may be deregistered, or stop sharing its kind of data,
//! while the batch is being written. After writing, the owner's connection
//! and grant are read again, and what was just written is purged when either
//! is gone: a withdrawal that committed before that read missed those rows,
//! and one that commits after it runs its own purge after them.

mod activities;
mod consent;
mod health;
/// Pure conversions from push records to Dravr's models.
mod mapping;
/// The shapes of the records Garmin pushes.
mod wire;

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::constants::oauth_providers::GARMIN;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DataSource, DeviceType, TenantId};
use pierre_database::RepositoryRegistry;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::webhook_owner::{resolve_webhook_owner, WebhookTarget};

use consent::{GRANT_ACTIVITY_EXPORT, GRANT_HEALTH_EXPORT};

/// The health sync asks whether the API pushes an athlete's wellness.
#[cfg(feature = "health-sync")]
pub(crate) use consent::shares_wellness;

/// The `source` marker on the owner's Garmin API data source.
///
/// The usr/pwd connection's health sync stores Garmin wellness under the same
/// provider key, `garmin`, through a data source with no marker. Every row the
/// API writes is filed under the marked source, so a purge of the API's
/// wellness reaches exactly those rows
/// (`ProviderDataRepository::purge_user_provider_source_wellness`).
pub(crate) const API_SOURCE_MARKER: &str = GARMIN;

/// A summary type Garmin pushes and Dravr ingests, by the key its batches
/// carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SummaryType {
    /// `activities`
    Activities,
    /// `activityDetails`
    ActivityDetails,
    /// `activityFiles`
    ActivityFiles,
    /// `dailies`
    Dailies,
    /// `sleeps`
    Sleeps,
    /// `hrv`
    Hrv,
    /// `userMetrics`
    UserMetrics,
    /// `bodyComps`
    BodyComps,
    /// `stressDetails`
    StressDetails,
    /// `skinTemp`
    SkinTemp,
    /// `deregistrations`
    Deregistrations,
    /// `userPermissionsChange`
    UserPermissionsChange,
}

impl SummaryType {
    /// Every summary type ingested.
    pub const ALL: [Self; 12] = [
        Self::Activities,
        Self::ActivityDetails,
        Self::ActivityFiles,
        Self::Dailies,
        Self::Sleeps,
        Self::Hrv,
        Self::UserMetrics,
        Self::BodyComps,
        Self::StressDetails,
        Self::SkinTemp,
        Self::Deregistrations,
        Self::UserPermissionsChange,
    ];

    /// The key a batch of this type carries, and the last segment of the URL
    /// it is pushed to.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Activities => "activities",
            Self::ActivityDetails => "activityDetails",
            Self::ActivityFiles => "activityFiles",
            Self::Dailies => "dailies",
            Self::Sleeps => "sleeps",
            Self::Hrv => "hrv",
            Self::UserMetrics => "userMetrics",
            Self::BodyComps => "bodyComps",
            Self::StressDetails => "stressDetails",
            Self::SkinTemp => "skinTemp",
            Self::Deregistrations => "deregistrations",
            Self::UserPermissionsChange => "userPermissionsChange",
        }
    }

    /// The summary type a batch key names, when Dravr ingests it.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// The permission the athlete must grant for Dravr to keep this type,
    /// `None` for the consent notices themselves.
    #[must_use]
    pub const fn required_grant(self) -> Option<&'static str> {
        match self {
            Self::Activities | Self::ActivityDetails | Self::ActivityFiles => {
                Some(GRANT_ACTIVITY_EXPORT)
            }
            Self::Dailies
            | Self::Sleeps
            | Self::Hrv
            | Self::UserMetrics
            | Self::BodyComps
            | Self::StressDetails
            | Self::SkinTemp => Some(GRANT_HEALTH_EXPORT),
            Self::Deregistrations | Self::UserPermissionsChange => None,
        }
    }
}

/// Why a pushed body is not a Garmin batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// Not JSON, or not one object holding one list.
    Malformed(&'static str),
    /// A list under a key Dravr does not ingest.
    UnknownType,
}

/// The summary type a pushed body carries and its records: Garmin posts one
/// object whose single key is the type and whose value is the list.
///
/// # Errors
///
/// Returns [`EnvelopeError`] when the body is not one object holding one list
/// under a summary type Dravr ingests.
pub fn parse_envelope(body: &[u8]) -> Result<(SummaryType, Vec<Value>), EnvelopeError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| EnvelopeError::Malformed("not JSON"))?;
    let Value::Object(map) = value else {
        return Err(EnvelopeError::Malformed("not a JSON object"));
    };
    let mut entries = map.into_iter();
    let (Some((key, records)), None) = (entries.next(), entries.next()) else {
        return Err(EnvelopeError::Malformed("not exactly one summary list"));
    };
    let kind = SummaryType::from_key(&key).ok_or(EnvelopeError::UnknownType)?;
    let Value::Array(records) = records else {
        return Err(EnvelopeError::Malformed("the summary is not a list"));
    };
    Ok((kind, records))
}

/// The platform user a record is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarminOwner {
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant their Garmin connection lives in.
    pub tenant_id: TenantId,
}

/// What fetching an activity's one-shot file answered.
#[derive(Debug)]
pub enum ActivityFile {
    /// The file's bytes.
    Bytes(Vec<u8>),
    /// Already fetched, or past its 24 hours: Garmin will not serve it again.
    Gone,
    /// The client's download budget is spent; try again after the wait.
    RateLimited(Duration),
    /// Larger than any one workout's file (the cap uploads are held to): not
    /// read past the cap, and not asked for again.
    TooLarge,
}

/// What the ingest needs from the server it runs in.
#[async_trait]
pub trait GarminPushHost: Send + Sync {
    /// Withdraw an owner whose Garmin registration Garmin says is gone.
    ///
    /// # Errors
    /// Returns an error when the disconnect failed and should be retried.
    async fn deregister(&self, owner: GarminOwner) -> AppResult<()>;

    /// Fetch an activity file from its one-shot `callback_url`, with the
    /// owner's own Garmin token.
    ///
    /// # Errors
    /// Returns an error when the fetch failed in a way worth retrying.
    async fn fetch_activity_file(
        &self,
        owner: GarminOwner,
        callback_url: &str,
    ) -> AppResult<ActivityFile>;

    /// `stored` records of `kind` landed for `owner`.
    async fn landed(&self, owner: GarminOwner, kind: SummaryType, stored: usize);
}

/// Why some records of a batch must be tried again later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retry {
    /// They wait on another push that may never come (an activity file whose
    /// summary was skipped, or has not arrived): after the last run the batch
    /// is settled with what it stored, not failed.
    Wait(String),
    /// They failed for now (a spent download budget): after the last run the
    /// batch is failed.
    Later(String),
}

impl Retry {
    /// Why, as the row's `last_error` records it.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Wait(why) | Self::Later(why) => why,
        }
    }
}

/// What one batch did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchOutcome {
    /// Records written.
    pub stored: usize,
    /// Records not kept: manual, a multisport parent, incomplete, or data the
    /// athlete no longer shares.
    pub skipped: usize,
    /// Records whose Garmin user names no owner here.
    pub unowned: usize,
    /// Records that could not be read as their type.
    pub unreadable: usize,
    /// Why some records must be tried again later, when any must.
    pub retry: Option<Retry>,
}

impl BatchOutcome {
    pub(crate) fn absorb(&mut self, other: Self) {
        self.stored += other.stored;
        self.skipped += other.skipped;
        self.unowned += other.unowned;
        self.unreadable += other.unreadable;
        // A record that failed for now outranks one that waits: the batch
        // must fail rather than settle if it is still owed at its last run.
        match (&self.retry, other.retry) {
            (None | Some(Retry::Wait(_)), Some(later @ Retry::Later(_))) => {
                self.retry = Some(later);
            }
            (None, wait @ Some(Retry::Wait(_))) => self.retry = wait,
            _ => {}
        }
    }
}

/// One owner's records of a batch, and what writing them needs.
pub(crate) struct OwnerBatch<'a> {
    pub repos: &'a RepositoryRegistry,
    pub host: &'a dyn GarminPushHost,
    pub owner: GarminOwner,
    pub received_at: DateTime<Utc>,
}

impl OwnerBatch<'_> {
    /// The owner's Garmin data source, created on first use: every health
    /// row and series point is filed under it.
    pub(crate) async fn data_source(&self) -> AppResult<String> {
        let source = DataSource {
            id: Uuid::new_v4().to_string(),
            user_id: self.owner.user_id.to_string(),
            provider: GARMIN.to_owned(),
            device_model: None,
            software_version: None,
            source: Some(API_SOURCE_MARKER.to_owned()),
            device_type: DeviceType::Watch,
            original_source_name: Some("Garmin Connect".to_owned()),
        };
        self.repos
            .data_sources
            .upsert_data_source(&self.owner.tenant_id, &source)
            .await
    }
}

/// Read every record as `T`, counting those that do not read.
pub(crate) fn records_as<T: DeserializeOwned>(
    records: Vec<Value>,
    outcome: &mut BatchOutcome,
) -> Vec<T> {
    records
        .into_iter()
        .filter_map(|record| {
            serde_json::from_value(record)
                .inspect_err(|_| outcome.unreadable += 1)
                .ok()
        })
        .collect()
}

/// Ingest one stored batch of `kind` received at `received_at`.
///
/// Records are grouped by Garmin user, each group resolved to its owner once
/// and written in the order Garmin listed them.
///
/// # Errors
///
/// Returns an error when a store write or an owner lookup failed; the batch
/// is then retried whole, which every write tolerates.
pub async fn ingest_batch(
    repos: &RepositoryRegistry,
    host: &dyn GarminPushHost,
    kind: SummaryType,
    records: Vec<Value>,
    received_at: DateTime<Utc>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    for (garmin_user, records) in group_by_user(records, &mut outcome) {
        let count = records.len();
        let Some(owner) = owner_of(repos, &garmin_user).await? else {
            outcome.unowned += count;
            continue;
        };
        if !consent::shares(repos, owner, kind).await? {
            info!(
                provider = GARMIN,
                summary_type = kind.key(),
                user_id = %owner.user_id,
                records = count,
                "Garmin push: the athlete no longer shares this data with Dravr; records dropped"
            );
            outcome.skipped += count;
            continue;
        }
        let batch = OwnerBatch {
            repos,
            host,
            owner,
            received_at,
        };
        let mut written = write_owner_records(&batch, kind, records).await?;
        if written.stored > 0 && !consent::still_shares(repos, owner, &garmin_user, kind).await? {
            written.skipped += written.stored;
            written.stored = 0;
        }
        if written.stored > 0 {
            host.landed(owner, kind, written.stored).await;
        }
        outcome.absorb(written);
    }
    Ok(outcome)
}

/// Write one owner's records of `kind`.
async fn write_owner_records(
    batch: &OwnerBatch<'_>,
    kind: SummaryType,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    match kind {
        SummaryType::Activities => activities::write_summaries(batch, records).await,
        SummaryType::ActivityDetails => activities::write_details(batch, records).await,
        SummaryType::ActivityFiles => activities::write_files(batch, records).await,
        SummaryType::Dailies => health::write_dailies(batch, records).await,
        SummaryType::Sleeps => health::write_sleeps(batch, records).await,
        SummaryType::Hrv => health::write_hrv(batch, records).await,
        SummaryType::UserMetrics => health::write_user_metrics(batch, records).await,
        SummaryType::BodyComps => health::write_body_comps(batch, records).await,
        SummaryType::StressDetails => health::write_stress(batch, records).await,
        SummaryType::SkinTemp => health::write_skin_temp(batch, records).await,
        SummaryType::Deregistrations => consent::deregister(batch, records.len()).await,
        SummaryType::UserPermissionsChange => consent::permissions_changed(batch, records).await,
    }
}

/// The records grouped by the Garmin user each names, users in first-seen
/// order. A record naming no user is unreadable.
fn group_by_user(records: Vec<Value>, outcome: &mut BatchOutcome) -> Vec<(String, Vec<Value>)> {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for record in records {
        let Some(user) = record
            .get("userId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .map(str::to_owned)
        else {
            outcome.unreadable += 1;
            continue;
        };
        if let Some(&at) = index.get(&user) {
            groups[at].1.push(record);
        } else {
            index.insert(user.clone(), groups.len());
            groups.push((user, vec![record]));
        }
    }
    groups
}

/// The owner a Garmin user id names, or `None` when the push is not theirs
/// to keep.
///
/// Only the athlete who holds the `garmin` token carrying the id is served:
/// Garmin is not a coaching platform, so an id reached through a coach's
/// link is not a Garmin API athlete. The log line names the outcome, never
/// the Garmin user id.
pub(crate) async fn owner_of(
    repos: &RepositoryRegistry,
    garmin_user: &str,
) -> AppResult<Option<GarminOwner>> {
    match resolve_webhook_owner(repos, GARMIN, garmin_user).await? {
        WebhookTarget::Owner(owner) if owner.read_through.is_none() => {
            let tenant_id = TenantId::parse_str(&owner.tenant_id).map_err(|e| {
                AppError::internal(format!("Garmin push owner carries a malformed tenant: {e}"))
            })?;
            Ok(Some(GarminOwner {
                user_id: owner.user_id,
                tenant_id,
            }))
        }
        WebhookTarget::Owner(_) => {
            warn!(
                provider = GARMIN,
                "Garmin push names an athlete reached only through a coach's link; dropped"
            );
            Ok(None)
        }
        WebhookTarget::Tombstoned { .. } => Ok(None),
        WebhookTarget::Nobody => {
            info!(
                provider = GARMIN,
                "Garmin push names a Garmin user no connection here holds; dropped"
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_envelope_names_its_summary_type_and_its_records() {
        let (kind, records) =
            parse_envelope(br#"{"sleeps":[{"userId":"u1"},{"userId":"u2"}]}"#).unwrap();
        assert_eq!(
            kind,
            SummaryType::Sleeps,
            "the single key names the summary type"
        );
        assert_eq!(records.len(), 2, "every record of the list is returned");
    }

    #[test]
    fn an_envelope_with_an_unknown_type_or_two_lists_is_refused() {
        assert_eq!(
            parse_envelope(br#"{"epochs":[]}"#).unwrap_err(),
            EnvelopeError::UnknownType,
            "a type Dravr does not ingest is unknown"
        );
        assert!(
            matches!(
                parse_envelope(br#"{"sleeps":[],"dailies":[]}"#),
                Err(EnvelopeError::Malformed(_))
            ),
            "two lists in one body are malformed"
        );
        assert!(
            matches!(
                parse_envelope(br#"{"sleeps":{}}"#),
                Err(EnvelopeError::Malformed(_))
            ),
            "a summary that is not a list is malformed"
        );
        assert!(
            matches!(parse_envelope(b"nope"), Err(EnvelopeError::Malformed(_))),
            "a body that is not JSON is malformed"
        );
    }

    #[test]
    fn every_summary_type_round_trips_through_its_key() {
        for kind in SummaryType::ALL {
            assert_eq!(
                SummaryType::from_key(kind.key()),
                Some(kind),
                "every key reads back as its type"
            );
        }
    }

    #[test]
    fn records_are_grouped_by_user_in_first_seen_order() {
        let mut outcome = BatchOutcome::default();
        let groups = group_by_user(
            vec![
                serde_json::json!({"userId": "b"}),
                serde_json::json!({"userId": "a"}),
                serde_json::json!({"userId": "b"}),
                serde_json::json!({"summaryId": "x"}),
            ],
            &mut outcome,
        );
        assert_eq!(groups.len(), 2, "one group per Garmin user");
        assert_eq!(groups[0].0, "b", "users keep their first-seen order");
        assert_eq!(
            groups[0].1.len(),
            2,
            "a user's records are grouped together"
        );
        assert_eq!(
            outcome.unreadable, 1,
            "a record naming no user is unreadable"
        );
    }
}
