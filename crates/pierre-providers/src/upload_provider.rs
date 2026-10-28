// ABOUTME: The athlete's uploaded .fit files read as a provider: the cached activities, and the series from the stored file
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
//! the per-sample series and the route from the stored file.
//!
//! An uploaded activity's id is the SHA-256 of its file followed by the
//! session's index in it ([`upload_activity_id`]), so the same file uploaded
//! twice names the same activities.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_database::repositories::{ActivityCacheRepository, UploadedActivityFileRepository};
use ring::digest::{digest, SHA256};
use uuid::Uuid;

use crate::cached_activity_reads::CachedActivities;
use crate::constants::oauth_providers::UPLOAD;
use crate::core::{ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig};
use crate::errors::{AppError, AppResult};
use crate::fit_file::{decode_bounded, FitActivityFile, Position};
use crate::models::StatsCoverage;
use crate::models::{Activity, Athlete, Stats, TenantId, TimeSeriesData};
use crate::pagination::{CursorPage, PaginationParams};

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

/// One athlete's uploaded activities, in one tenant, read as a provider.
pub struct UploadProvider {
    config: ProviderConfig,
    cache: Arc<dyn ActivityCacheRepository>,
    files: Arc<dyn UploadedActivityFileRepository>,
    user_id: Uuid,
    tenant_id: TenantId,
}

impl fmt::Debug for UploadProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadProvider")
            .field("config", &self.config)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}

impl UploadProvider {
    /// The reader of `user_id`'s uploads in `tenant_id`.
    #[must_use]
    pub fn new(
        cache: Arc<dyn ActivityCacheRepository>,
        files: Arc<dyn UploadedActivityFileRepository>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            // Nothing is fetched: there is no endpoint, scope or token.
            config: ProviderConfig {
                name: UPLOAD.to_owned(),
                auth_url: String::new(),
                token_url: String::new(),
                api_base_url: String::new(),
                revoke_url: None,
                default_scopes: Vec::new(),
            },
            cache,
            files,
            user_id,
            tenant_id,
        }
    }

    /// The uploads' cached rows, which every list and single read answers
    /// from.
    fn cached(&self) -> CachedActivities<'_> {
        CachedActivities {
            cache: self.cache.as_ref(),
            user_id: self.user_id,
            tenant_id: &self.tenant_id,
            provider: UPLOAD,
        }
    }

    /// The stored file behind an uploaded activity, decoded, with the index
    /// of the activity's session in it.
    async fn file_of(&self, id: &str) -> AppResult<(FitActivityFile, usize)> {
        let not_found = || AppError::not_found(format!("uploaded activity {id}"));
        let (sha, index) = parse_upload_activity_id(id).ok_or_else(not_found)?;
        let bytes = self
            .files
            .get_uploaded_file(&self.tenant_id, self.user_id, sha)
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
        Ok((file, index))
    }
}

#[async_trait]
impl FitnessProvider for UploadProvider {
    fn name(&self) -> &'static str {
        UPLOAD
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Err(AppError::invalid_input(
            "uploaded activities are read without credentials",
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
            "an athlete profile (an uploaded file carries none)",
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
        self.cached().one(id, "uploaded activity").await
    }

    fn serves_activity_streams(&self) -> bool {
        true
    }

    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let (file, index) = self.file_of(id).await?;
        file.activity_with_series(index, id.to_owned(), UPLOAD, Position::Read)
            .ok_or_else(|| AppError::not_found(format!("uploaded activity {id}")))
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        let (file, index) = self.file_of(id).await?;
        Ok(file.session_series(index, Position::Read))
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
