// ABOUTME: Activity reads answered from the activity cache: a window, a cursor page, one activity, and the totals
// ABOUTME: Shared by every provider read from Dravr's own stores, with the one store-backed per-activity sample read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity reads over one provider key's cached rows.
//!
//! A provider read from Dravr's own stores rather than over the network —
//! the athlete's uploaded `.fit` files ([`crate::upload_provider`]) and a
//! provider that pushes its data ([`crate::spi::ProviderCapabilities::PUSH_ONLY`])
//! — answers its list, cursor and single reads from the rows cached under its
//! key. This is that read, written once, scoped to one user in one tenant.

use chrono::{DateTime, Utc};
use pierre_database::repositories::ActivityCacheRepository;
use serde_json::Value;
use uuid::Uuid;

use crate::core::{ActivityQueryParams, OwnerStores};
use crate::errors::{AppError, AppResult};
use crate::models::{Activity, TenantId, TimeSeriesData};
use crate::pagination::{Cursor, CursorPage, PaginationParams};

/// Most activities one list read returns when it names no limit.
const DEFAULT_LIST_LIMIT: usize = 200;

/// Rows a cursor page reads past its own size, for the activities that share
/// the cursor's start second.
const SAME_START_SLACK: usize = 16;

/// One provider key's cached activities, for one user in one tenant.
pub(crate) struct CachedActivities<'a> {
    /// The activity cache.
    pub(crate) cache: &'a dyn ActivityCacheRepository,
    /// The rows' owner.
    pub(crate) user_id: Uuid,
    /// The tenant the rows were written in.
    pub(crate) tenant_id: &'a TenantId,
    /// The provider key the rows are cached under.
    pub(crate) provider: &'a str,
}

impl CachedActivities<'_> {
    /// The activities inside `params`' window, newest first, `offset` rows
    /// skipped and at most `limit` ([`DEFAULT_LIST_LIMIT`] when it names
    /// none) returned.
    pub(crate) async fn window(&self, params: &ActivityQueryParams) -> AppResult<Vec<Activity>> {
        let start = params
            .after
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
            .unwrap_or(DateTime::UNIX_EPOCH);
        let end = params
            .before
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
            .unwrap_or_else(Utc::now);
        let offset = params.offset.unwrap_or(0);
        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        let read = i64::try_from(offset.saturating_add(limit)).unwrap_or(i64::MAX);
        let rows = self
            .cache
            .get_cached_activities(
                self.user_id,
                self.tenant_id,
                Some(self.provider),
                start,
                end,
                read,
            )
            .await?;
        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }

    /// Newest first, a page at a time. The cursor names the last activity
    /// served by its start and id; the next page is what sorts after it in
    /// (start, id) descending order. Activities that share the cursor's start
    /// second were read as one batch, so the read reaches
    /// [`SAME_START_SLACK`] rows past the page to find them.
    pub(crate) async fn cursor_page(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let after = params.cursor.as_ref().and_then(Cursor::decode);
        let end = after.as_ref().map_or_else(Utc::now, |(at, _)| *at);
        let read = params.limit.saturating_add(1 + SAME_START_SLACK);
        let mut rows = self
            .cache
            .get_cached_activities(
                self.user_id,
                self.tenant_id,
                Some(self.provider),
                DateTime::UNIX_EPOCH,
                end,
                i64::try_from(read).unwrap_or(i64::MAX),
            )
            .await?;
        rows.sort_by(|a, b| {
            b.start_date()
                .cmp(&a.start_date())
                .then_with(|| b.id().cmp(a.id()))
        });
        let mut page: Vec<Activity> = rows
            .into_iter()
            .filter(|row| {
                after.as_ref().is_none_or(|(at, id)| {
                    row.start_date() < *at || (row.start_date() == *at && row.id() < id.as_str())
                })
            })
            .take(params.limit.saturating_add(1))
            .collect();
        let has_more = page.len() > params.limit;
        page.truncate(params.limit);
        let next_cursor = has_more
            .then(|| page.last())
            .flatten()
            .map(|last| Cursor::new(last.start_date(), last.id()));
        let count = page.len();
        Ok(CursorPage {
            items: page,
            next_cursor,
            prev_cursor: None,
            has_more,
            count,
        })
    }

    /// One cached activity by id; not found when the user holds no such row
    /// under this provider key in this tenant. `what` names the activity in
    /// the error ("uploaded activity", "Garmin activity").
    pub(crate) async fn one(&self, id: &str, what: &str) -> AppResult<Activity> {
        self.cache
            .get_cached_activity(self.user_id, self.tenant_id, self.provider, id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("{what} {id}")))
    }

    /// Every cached activity under this provider key, newest first: what a
    /// totals read sums over.
    pub(crate) async fn all(&self) -> AppResult<Vec<Activity>> {
        self.window(&ActivityQueryParams::with_pagination(
            Some(usize::MAX / 2),
            None,
        ))
        .await
    }
}

/// The store-backed half of the per-activity sample reads
/// ([`crate::core::FitnessProvider::get_activity_streams`],
/// [`crate::core::FitnessProvider::get_activity_with_streams`]) for a
/// provider whose samples Dravr holds in `activity_streams` rather than
/// reading them from the provider: written once here, keyed by the provider
/// slug the activity is read under, over the stores bound to one connection's
/// owner.
impl OwnerStores {
    /// The samples stored for the owner's activity `id` under `provider`,
    /// `None` when none were stored.
    ///
    /// # Errors
    ///
    /// A failed or unreadable store read.
    pub async fn stored_activity_streams(
        &self,
        provider: &str,
        id: &str,
    ) -> AppResult<Option<TimeSeriesData>> {
        self.activity_streams
            .get_activity_streams(&self.tenant_id, self.user_id, provider, id)
            .await
    }

    /// The owner's cached activity `id` under `provider`, with its stored
    /// samples attached when there are any. An activity whose samples were
    /// never stored is served without them, which says nothing about whether
    /// it recorded any. `what` names the activity in a not-found error.
    ///
    /// # Errors
    ///
    /// Not found when the owner holds no such activity under `provider`; a
    /// failed or unreadable store read.
    pub async fn stored_activity_with_streams(
        &self,
        provider: &str,
        id: &str,
        what: &str,
    ) -> AppResult<Activity> {
        let activity = CachedActivities {
            cache: self.activity_cache.as_ref(),
            user_id: self.user_id,
            tenant_id: &self.tenant_id,
            provider,
        }
        .one(id, what)
        .await?;
        match self.stored_activity_streams(provider, id).await? {
            Some(streams) => with_streams(&activity, &streams),
            None => Ok(activity),
        }
    }
}

/// `activity` with `streams` attached.
///
/// A built [`Activity`] takes no samples, and the cached copy is read back
/// built, so they are attached through its serialized form, under the field
/// the model serializes them as.
pub(crate) fn with_streams(activity: &Activity, streams: &TimeSeriesData) -> AppResult<Activity> {
    let mut value = serde_json::to_value(activity)
        .map_err(|e| AppError::internal(format!("serialize cached activity: {e}")))?;
    let streams = serde_json::to_value(streams)
        .map_err(|e| AppError::internal(format!("serialize stored samples: {e}")))?;
    let Value::Object(fields) = &mut value else {
        return Err(AppError::internal(
            "a cached activity does not serialize as an object",
        ));
    };
    fields.insert("time_series_data".to_owned(), streams);
    serde_json::from_value(value)
        .map_err(|e| AppError::internal(format!("attach stored samples: {e}")))
}
