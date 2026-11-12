// ABOUTME: The one way a provider's data is purged: the repository's tables, and every copy kept beside them
// ABOUTME: A port: pierre-services implements it over the repository and the activity store, so leaf crates can purge

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider data purge, as its callers see it.
//!
//! [`ProviderDataRepository`](super::ProviderDataRepository) deletes the rows
//! a provider contributed to the platform's tables and attests the deletion.
//! A provider's activities are also kept as records in the activity store
//! (dravr-barrage), which projects them into those tables; a purge that left
//! the record would leave the athlete's data held after a disconnect, and a
//! later projection could serve it again (carnet#1482).
//!
//! [`ProviderPurge`] is the purge every caller makes: the same operations,
//! with the same attestation, plus the activity store's copy of a live
//! source. It is declared here, beside the repository it wraps, so a crate
//! below the services (`pierre-groups`) purges through it with no dependency
//! on the activity store; `pierre_services::provider_purge` implements it.
//! An imported archive is never purged with a live disconnect: its records
//! go only when the athlete deletes the import.

use std::fmt;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

use super::{ProviderDataPurge, PurgeReason};

/// Deletes what a provider contributed, wherever the platform keeps it.
///
/// Each operation is the [`ProviderDataRepository`](super::ProviderDataRepository)
/// purge of the same name, which writes the one attestation row, followed by
/// the deletion of the activity store's records of the provider when the
/// operation deletes its activities.
#[async_trait]
pub trait ProviderPurge: Send + Sync + fmt::Debug {
    /// Delete every row `provider` contributed for `user_id` under
    /// `tenant_id`, and the activity store's records of it, attesting
    /// `reason` once.
    ///
    /// # Errors
    /// Returns a database error when the purge fails, or a store error when
    /// the records cannot be deleted; a retry deletes what is left.
    async fn purge_user_provider_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete the wellness `provider` contributed for `user_id` under
    /// `tenant_id`, attesting `reason`. The provider's activities stay, in
    /// the tables and in the activity store.
    ///
    /// # Errors
    /// Returns a database error when the purge fails; nothing is deleted then.
    async fn purge_user_provider_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete the activities `provider` contributed for `user_id` under
    /// `tenant_id`, with the sync state describing them, and the activity
    /// store's records of it, attesting `reason` once. Its wellness stays.
    ///
    /// # Errors
    /// As [`Self::purge_user_provider_data`].
    async fn purge_user_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete the wellness `provider` contributed for `user_id` under
    /// `tenant_id` through its data sources marked `source`, attesting
    /// `reason`. Its activities stay.
    ///
    /// # Errors
    /// Returns a database error when the purge fails; nothing is deleted then.
    async fn purge_user_provider_source_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete `provider`'s activities for `user_id` under `tenant_id` and
    /// the wellness filed under its data sources marked `source`, and the
    /// activity store's records of it, attesting `reason` once.
    ///
    /// # Errors
    /// As [`Self::purge_user_provider_data`].
    async fn purge_user_provider_source_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete every row `provider` contributed, for every user in every
    /// tenant, and every activity store record of it: the operator's
    /// termination purge.
    ///
    /// # Errors
    /// As [`Self::purge_user_provider_data`].
    async fn purge_provider_data(&self, provider: &str) -> AppResult<ProviderDataPurge>;
}
