// ABOUTME: Activities the athlete brought in (uploaded .fit files, imported archives) read as a provider
// ABOUTME: No connection, token or network — every read is the owner's own rows in their own tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Uploaded activity files as a provider
//!
//! An athlete who uploads the `.fit` of a completed workout (carnet#818) gets
//! one activity per session in it, cached under the [`UPLOAD`] key like any
//! provider's copy, and the file kept beside it. Every reader that addresses
//! an activity by its provider — the activity's view and its route, the
//! agent's per-activity tools — resolves a provider by that key through the
//! authentication chokepoint, which answers [`UPLOAD`] with this reader
//! instead of a connection: the summary and laps come from the cached row,
//! the per-sample series and the route from the one per-activity sample store
//! (`activity_streams`), where the upload wrote them once, decoded, with
//! `origin = 'upload'` and the file's SHA-256 as `source_ref` (carnet#1219).
//!
//! An uploaded activity whose samples the store does not hold at its current
//! format version — one uploaded before the store held uploads, or stored
//! before GPS and distance kept their gaps, until the backfill reaches it —
//! is read from the stored file, decoded again on the blocking pool: the
//! store is the fast path, the file stays the source both are decoded from.
//!
//! An uploaded activity's id is the SHA-256 of its file followed by the
//! session's index in it ([`upload_activity_id`]), so the same file uploaded
//! twice names the same activities.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dravr_barrage::parse::fit::{FitActivityFile, Position};
use pierre_database::repositories::{ActivityStreamRepository, ActivityStreamsWrite, StreamOrigin};
use ring::digest::{digest, SHA256};
use uuid::Uuid;

use crate::cached_activity_reads::{with_streams, CachedActivities};
use crate::constants::oauth_providers::UPLOAD;
use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
};
use crate::errors::{AppError, AppResult};
use crate::fit_decode::decode_bounded;
use crate::models::StatsCoverage;
use crate::models::{Activity, Athlete, Stats, TenantId, TimeSeriesData};
use crate::originals::OriginalsStore;
use crate::pagination::{CursorPage, PaginationParams};

/// What a not-found error calls an uploaded activity.
const UPLOADED_ACTIVITY: &str = "uploaded activity";

/// What a not-found error calls an activity from an imported archive.
const IMPORTED_ACTIVITY: &str = "imported activity";

/// The hex SHA-256 of an uploaded file's bytes: the key the file is kept
/// under.
#[must_use]
pub fn file_sha256(bytes: &[u8]) -> String {
    hex::encode(digest(&SHA256, bytes))
}

/// The id of the activity a file's session at `index` becomes.
#[must_use]
pub fn upload_activity_id(file_sha256: &str, index: usize) -> String {
    format!("{file_sha256}-{index}")
}

/// The file hash and session index an uploaded activity's id names, or
/// `None` for an id that is not one.
#[must_use]
pub fn parse_upload_activity_id(id: &str) -> Option<(&str, usize)> {
    let (sha, index) = id.rsplit_once('-')?;
    let well_formed = sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    well_formed.then_some(())?;
    Some((sha, index.parse().ok()?))
}

/// The series of each session of `file` named in `indices`, with its index.
///
/// What an upload stores for the sessions it keeps. A session the file does
/// not hold is left out. CPU-bound, so it runs where the file was decoded, on
/// the blocking pool.
#[must_use]
pub fn upload_session_series(
    file: &FitActivityFile,
    indices: impl IntoIterator<Item = usize>,
) -> Vec<(usize, TimeSeriesData)> {
    indices
        .into_iter()
        .filter_map(|index| Some((index, file.session_series(index, Position::Read)?)))
        .collect()
}

/// Store each session series of the uploaded file `file_sha256`.
///
/// Each goes to the one per-activity sample store, under the activity id
/// that session became, with `origin = 'upload'` and the file's hash as
/// `source_ref`.
///
/// Idempotent: the same series written again replaces the row with itself.
///
/// # Errors
/// Returns a database error when a write fails.
pub async fn store_upload_series(
    streams: &dyn ActivityStreamRepository,
    tenant_id: &TenantId,
    user_id: Uuid,
    file_sha256: &str,
    series: &[(usize, TimeSeriesData)],
    synced_at: DateTime<Utc>,
) -> AppResult<()> {
    for (index, samples) in series {
        let activity_id = upload_activity_id(file_sha256, *index);
        streams
            .upsert_activity_streams(&ActivityStreamsWrite {
                tenant_id,
                user_id,
                provider: UPLOAD,
                activity_id: &activity_id,
                origin: StreamOrigin::Upload,
                source_ref: Some(file_sha256),
                streams: samples,
                synced_at,
            })
            .await?;
    }
    Ok(())
}

/// One athlete's activities under a key they brought in themselves — their
/// uploaded files ([`UPLOAD`]) or an imported archive (`strava_archive`) — in
/// one tenant, read as a provider.
///
/// Lists, single reads and stats answer from the cached rows filed under the
/// key; samples from the one per-activity sample store. An uploaded activity
/// whose samples the store does not hold at its current version is decoded
/// again from its kept file. An archive's samples were projected into the
/// store when it was imported, so one the store does not hold has none Dravr
/// could decode (a manual entry, a recording format not read yet), and the
/// read says it did not reach any (`None`).
pub struct UserProvidedProvider {
    key: &'static str,
    config: ProviderConfig,
    stores: OwnerStores,
    files: Option<Arc<OriginalsStore>>,
}

impl fmt::Debug for UserProvidedProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserProvidedProvider")
            .field("key", &self.key)
            .field("config", &self.config)
            .field("stores", &self.stores)
            .finish_non_exhaustive()
    }
}

impl UserProvidedProvider {
    /// The reader of the uploads of the owner `stores` are bound to, in
    /// their tenant.
    #[must_use]
    pub fn upload(stores: OwnerStores, files: Arc<OriginalsStore>) -> Self {
        Self::with(UPLOAD, stores, Some(files))
    }

    /// The reader of the owner's imported archive `key`
    /// (`USER_PROVIDED_PROVIDERS` other than [`UPLOAD`]), in their tenant.
    #[must_use]
    pub fn archive(key: &'static str, stores: OwnerStores) -> Self {
        Self::with(key, stores, None)
    }

    /// The reader of the user-provided `key`: the uploads' with their kept
    /// files, any other key's without.
    #[must_use]
    pub fn for_key(key: &'static str, stores: OwnerStores, files: Arc<OriginalsStore>) -> Self {
        if key == UPLOAD {
            Self::upload(stores, files)
        } else {
            Self::archive(key, stores)
        }
    }

    fn with(key: &'static str, stores: OwnerStores, files: Option<Arc<OriginalsStore>>) -> Self {
        Self {
            key,
            // Nothing is fetched: there is no endpoint, scope or token.
            config: ProviderConfig {
                name: key.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: Vec::new(),
            },
            stores,
            files,
        }
    }

    /// The key's cached rows, which every list and single read answers from.
    fn cached(&self) -> CachedActivities<'_> {
        CachedActivities {
            cache: self.stores.activity_cache.as_ref(),
            user_id: self.stores.user_id,
            tenant_id: &self.stores.tenant_id,
            provider: self.key,
        }
    }

    /// The stored file behind an uploaded activity, decoded, with the index
    /// of the activity's session in it: the read of an activity whose
    /// samples the store does not hold yet. `None` for a key that keeps no
    /// files here (an archive's originals are barrage's).
    async fn file_of(&self, id: &str) -> AppResult<Option<(FitActivityFile, usize)>> {
        let Some(files) = &self.files else {
            return Ok(None);
        };
        let not_found = || AppError::not_found(format!("{UPLOADED_ACTIVITY} {id}"));
        let (sha, index) = parse_upload_activity_id(id).ok_or_else(not_found)?;
        let bytes = files
            .get_upload(&self.stores.tenant_id, self.stores.user_id, sha)
            .await?
            .ok_or_else(not_found)?;
        // The decode is CPU-bound and memory-heavy: it runs on the blocking
        // pool under the process-wide decode bound.
        let file = decode_bounded(move || FitActivityFile::parse(&bytes))
            .await
            .map_err(|e| AppError::internal(format!("decoding stored upload {sha}: {e}")))?
            .map_err(|e| {
                AppError::internal(format!("stored upload {sha} no longer decodes: {e}"))
            })?;
        Ok(Some((file, index)))
    }

    /// What a not-found error calls one of this key's activities.
    const fn what(&self) -> &'static str {
        if self.files.is_some() {
            UPLOADED_ACTIVITY
        } else {
            IMPORTED_ACTIVITY
        }
    }
}

#[async_trait]
impl FitnessProvider for UserProvidedProvider {
    fn name(&self) -> &'static str {
        self.key
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Err(AppError::invalid_input(
            "activities the athlete brought in are read without credentials",
        ))
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::not_found(
            "an athlete profile (an uploaded file or an imported archive carries none)",
        ))
    }

    async fn get_activities(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<Activity>> {
        self.get_activities_with_params(&ActivityQueryParams::with_pagination(limit, offset))
            .await
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        self.cached().window(params).await
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        self.cached().cursor_page(params).await
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        self.cached().one(id, self.what()).await
    }

    fn serves_activity_streams(&self) -> bool {
        true
    }

    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        if let Some(streams) = self.stores.stored_activity_streams(self.key, id).await? {
            let activity = self.cached().one(id, self.what()).await?;
            return with_streams(&activity, &streams);
        }
        match self.file_of(id).await? {
            Some((file, index)) => file
                .activity_with_series(index, id.to_owned(), self.key, Position::Read)
                .ok_or_else(|| AppError::not_found(format!("{} {id}", self.what()))),
            None => self.cached().one(id, self.what()).await,
        }
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        if let Some(streams) = self.stores.stored_activity_streams(self.key, id).await? {
            return Ok(Some(streams));
        }
        Ok(self
            .file_of(id)
            .await?
            .and_then(|(file, index)| file.session_series(index, Position::Read)))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        let activities = self.cached().all().await?;
        Ok(Stats {
            total_activities: u64::try_from(activities.len()).unwrap_or(u64::MAX),
            total_distance: activities
                .iter()
                .filter_map(Activity::distance_meters)
                .sum(),
            total_duration: activities.iter().map(Activity::duration_seconds).sum(),
            total_elevation_gain: activities.iter().filter_map(Activity::elevation_gain).sum(),
            coverage: StatsCoverage::Lifetime,
            year_to_date: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_upload_id_names_its_file_and_session() {
        let sha = file_sha256(b"a ride");
        assert_eq!(sha.len(), 64);
        let id = upload_activity_id(&sha, 2);
        assert_eq!(parse_upload_activity_id(&id), Some((sha.as_str(), 2)));
    }

    #[test]
    fn an_id_that_is_not_an_upload_names_nothing() {
        for id in ["12345", "abc-0", "", "-1"] {
            assert_eq!(parse_upload_activity_id(id), None, "{id}");
        }
        let sha = file_sha256(b"x");
        assert_eq!(parse_upload_activity_id(&format!("{sha}-x")), None);
        assert_eq!(parse_upload_activity_id(&sha), None);
    }
}
