// ABOUTME: The one write path of every provider's activities: each read, push or upload goes into the activity store
// ABOUTME: The server implements it over barrage, which keeps the record and projects it into the cache readers read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live activity writes.
//!
//! Every provider read the platform makes — the chat turn's live read, a
//! backfill, a push, a webhook, an upload — lands its activities through
//! [`LiveActivityStore`], and every deletion of one goes through it too. The
//! server's implementation keeps each one in dravr-barrage, the athlete's
//! activity store, under one retention policy, and projects it into
//! `cached_activities`, which every reader reads (carnet#1482).

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use uuid::Uuid;

/// What one live ingest did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveIngest {
    /// Rows held after the write: inserted, updated or unchanged. A row the
    /// athlete deleted is not one of them.
    pub stored: u64,
    /// Rows the write inserted: the store held no record of them before it.
    pub inserted: u64,
    /// Rows the write refused: the athlete deleted them ([`LiveActivityStore::erase`]).
    pub refused: u64,
    /// Whether a session newer than every activity held landed.
    pub newer_session: bool,
}

/// The write path of every provider's activities.
#[async_trait]
pub trait LiveActivityStore: Send + Sync {
    /// Keep `activities`, read from `provider` for the athlete, at the sync
    /// stamp `synced_at`, and serve them to the cache readers. An activity
    /// the athlete deleted ([`Self::erase`]) is neither kept nor served.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source the store knows,
    /// or the store or its projection fails.
    async fn ingest(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
        synced_at: DateTime<Utc>,
    ) -> AppResult<LiveIngest>;

    /// Keep one pushed `activity`, received at `received_at`, unless a copy
    /// of it received later is already held: a push retried after a newer
    /// one leaves the newer copy standing. Returns whether it was kept.
    ///
    /// # Errors
    ///
    /// As [`Self::ingest`].
    async fn ingest_received(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
        received_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Rewrite the content of the athlete's standing `activity` of
    /// `provider` in place, keeping its receipt: a figure derived from data
    /// stored after the activity's own write (a pushed ride's power, from
    /// samples that landed later) belongs on whichever copy stands, received
    /// before or after those samples. The store's record and the cached row
    /// change together. One the cache served before the store held the
    /// provider's reads is amended in the cache alone. Returns whether a copy
    /// stood.
    ///
    /// # Errors
    ///
    /// As [`Self::ingest`].
    async fn amend(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
    ) -> AppResult<bool>;

    /// The athlete or the provider deleted the workout `activity_id`: it
    /// stops being served (its cached row, samples and gear attribution
    /// go), the store forgets it, and no later read of `provider` brings it
    /// back — one never read yet included, as when a delete webhook outruns
    /// the sync that would have stored it. Returns whether the store or the
    /// cache held it.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, the id is not
    /// one the store can hold, or the store or its projection fails.
    async fn erase(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool>;

    /// Drop the store's copy of `activity_id` without refusing it later:
    /// the athlete deleted a file they uploaded and may upload it again.
    /// It stops being served first. Returns whether the store held it.
    ///
    /// # Errors
    ///
    /// As [`Self::erase`].
    async fn discard(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool>;

    /// Drop every copy of `provider`'s activities for the athlete that
    /// started within `[start, end]`, both ends included, as [`Self::discard`]
    /// drops one: a list read of that window vouched the provider no longer
    /// holds them, and a later read that shows one again keeps it again. The
    /// cache's rows in the window and the store's records there both go, so
    /// the two never disagree. Returns how many activities were dropped.
    ///
    /// Each activity goes on its own: a failure part-way leaves the ones
    /// before it dropped and returns the error, and a retry drops the rest.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the cache,
    /// the store or its projection fails.
    async fn discard_between(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<u64>;

    /// How many records the store holds of `provider` for the athlete: what
    /// an adoption of the cache checks the cache's count against.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the store
    /// fails.
    async fn held(&self, user_id: Uuid, tenant_id: TenantId, provider: &str) -> AppResult<u64>;

    /// The ids among `activities` the athlete's `provider` rows already
    /// hold, read in one pass over the window their start dates span: a
    /// write of them adds the rest.
    ///
    /// # Errors
    ///
    /// Returns an error when the rows cannot be read.
    async fn held_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>>;

    /// The ids among `activities` the store keeps a record of under the
    /// athlete's `provider`, read in one pass over the window their start
    /// dates span. Unlike [`Self::held_among`], a row the cache served
    /// before the store held the provider's reads is not one of them: a
    /// write of `activities` inserts exactly those it does not name, the
    /// athlete's deleted ones aside.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the store
    /// fails.
    async fn recorded_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>>;

    /// Delete everything the store holds of `provider` for the athlete — a
    /// disconnect or a purge — after its cached rows stop being served, and
    /// forget what the athlete had deleted of it, so a reconnection starts
    /// over. What the platform kept beside those rows (routes, pushed
    /// samples, best efforts, gear, sync state) and the purge's attestation
    /// are the provider purge's (`pierre_services::provider_purge`), which
    /// calls this after it: on its own this is not a whole disconnect.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the store or
    /// its projection fails.
    async fn delete_source(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;

    /// Whether `provider` is a live source the store keeps records of. An
    /// imported archive is not one (its records arrive through an import and
    /// go only with it), nor is a key the store does not know.
    fn keeps(&self, provider: &str) -> bool;

    /// Delete everything the store holds of `provider`, for every athlete in
    /// every tenant, as [`Self::delete_source`] does for one: the operator's
    /// termination purge. Returns how many athletes' records were deleted.
    ///
    /// # Errors
    ///
    /// As [`Self::delete_source`].
    async fn delete_source_everywhere(&self, provider: &str) -> AppResult<u64>;

    /// Delete every record the store holds of the athlete, of every live
    /// source in every tenant, and what they had deleted: the account's
    /// delete or reset. Returns how many records were deleted.
    ///
    /// The cached rows are not retracted here: the account delete and the
    /// reset wipe the athlete's cache themselves, and this runs beside that
    /// wipe so the store holds nothing a later sync would serve again.
    ///
    /// # Errors
    ///
    /// Returns an error when the store fails.
    async fn delete_user(&self, user_id: Uuid) -> AppResult<u64>;
}
