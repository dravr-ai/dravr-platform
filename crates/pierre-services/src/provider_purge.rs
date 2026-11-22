// ABOUTME: The provider data purge every caller makes: the repository's tables, then the activity store's live records
// ABOUTME: One attestation per purge, the repository's; an imported archive is never purged with a live disconnect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! [`ProviderPurge`] over the repository and the live activity store.
//!
//! A purge that deletes a provider's activities deletes them in two stores:
//! the platform's tables, through [`ProviderDataRepository`], which writes the
//! purge's one attestation row naming the caller's reason; then the activity
//! store's records of the provider ([`LiveActivityStore::delete_source`]),
//! whose projection attests nothing. The two stores share no transaction.
//! The tables go first, so the attestation counts every row they held; a
//! failure after them leaves the records for a retried purge, which finds the
//! tables empty and deletes the records.
//!
//! Only a live source's records are deleted: a provider key the store does
//! not keep (an imported archive, or a key it does not know) is purged from
//! the tables alone. Wellness-only purges never reach the store, which holds
//! activities.
//!
//! A purge of the `lab_document` provider ([`LAB_DOCUMENT_PROVIDER`]) deletes
//! the PDFs the athlete uploaded first, from the originals store, then the
//! tables delete the documents' rows with the records read from them: bytes
//! first, so a failure leaves the rows for a retried purge to find, as
//! deleting one lab document does.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    HealthRecordRepository, ProviderDataPurge, ProviderDataRepository, ProviderPurge, PurgeReason,
    LAB_DOCUMENT_PROVIDER, LAB_DOCUMENT_SOURCE_MARKER,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::originals::OriginalsStore;
use tracing::info;
use uuid::Uuid;

use crate::live_activities::LiveActivityStore;

/// Purges a provider's data from the tables and the live activity store.
#[derive(Clone)]
pub struct ProviderPurgeService {
    data: Arc<dyn ProviderDataRepository>,
    live: Arc<dyn LiveActivityStore>,
    health_records: Arc<dyn HealthRecordRepository>,
    originals: Arc<OriginalsStore>,
}

impl fmt::Debug for ProviderPurgeService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderPurgeService")
            .finish_non_exhaustive()
    }
}

impl ProviderPurgeService {
    /// The purge over `repos`' provider data, the live store `live` and the
    /// uploaded files in `originals`, as every holder of the port takes it.
    #[must_use]
    pub fn shared(
        repos: &RepositoryRegistry,
        live: Arc<dyn LiveActivityStore>,
        originals: Arc<OriginalsStore>,
    ) -> Arc<dyn ProviderPurge> {
        Arc::new(Self {
            data: Arc::clone(&repos.provider_data),
            live,
            health_records: Arc::clone(&repos.health_records),
            originals,
        })
    }

    /// Delete the PDFs of the athlete's lab documents when `provider` is
    /// [`LAB_DOCUMENT_PROVIDER`], before the tables delete their rows.
    async fn delete_lab_files(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<()> {
        if provider != LAB_DOCUMENT_PROVIDER {
            return Ok(());
        }
        let documents = self
            .health_records
            .list_lab_documents(tenant_id, user_id)
            .await?;
        for document in &documents {
            self.originals
                .delete_named_file(tenant_id, user_id, &document.file_sha256)
                .await?;
        }
        info!(%user_id, %tenant_id, files = documents.len(), "Deleted the purged lab documents' files");
        Ok(())
    }

    /// Delete the store's records of `provider` for the athlete, when it is
    /// a live source the store keeps.
    async fn delete_live_records(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<()> {
        if !self.live.keeps(provider) {
            return Ok(());
        }
        self.live
            .delete_source(user_id, *tenant_id, provider)
            .await?;
        info!(%user_id, %tenant_id, provider, "Deleted the activity store's records of the purged provider");
        Ok(())
    }
}

#[async_trait]
impl ProviderPurge for ProviderPurgeService {
    async fn purge_user_provider_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        self.delete_lab_files(user_id, tenant_id, provider).await?;
        let purge = self
            .data
            .purge_user_provider_data(user_id, tenant_id, provider, reason)
            .await?;
        self.delete_live_records(user_id, tenant_id, provider)
            .await?;
        Ok(purge)
    }

    async fn purge_user_provider_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        self.delete_lab_files(user_id, tenant_id, provider).await?;
        self.data
            .purge_user_provider_wellness(user_id, tenant_id, provider, reason)
            .await
    }

    async fn purge_user_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        let purge = self
            .data
            .purge_user_provider_activities(user_id, tenant_id, provider, reason)
            .await?;
        self.delete_live_records(user_id, tenant_id, provider)
            .await?;
        Ok(purge)
    }

    async fn purge_user_provider_source_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        // The statements delete lab documents only under their own source.
        if source == LAB_DOCUMENT_SOURCE_MARKER {
            self.delete_lab_files(user_id, tenant_id, provider).await?;
        }
        self.data
            .purge_user_provider_source_wellness(user_id, tenant_id, provider, source, reason)
            .await
    }

    async fn purge_user_provider_source_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        // The statements delete lab documents only under their own source.
        if source == LAB_DOCUMENT_SOURCE_MARKER {
            self.delete_lab_files(user_id, tenant_id, provider).await?;
        }
        let purge = self
            .data
            .purge_user_provider_source_data(user_id, tenant_id, provider, source, reason)
            .await?;
        self.delete_live_records(user_id, tenant_id, provider)
            .await?;
        Ok(purge)
    }

    async fn purge_provider_data(&self, provider: &str) -> AppResult<ProviderDataPurge> {
        if provider == LAB_DOCUMENT_PROVIDER {
            let files = self.health_records.list_every_lab_document_file().await?;
            for file in &files {
                self.originals
                    .delete_named_file(&file.tenant_id, file.user_id, &file.file_sha256)
                    .await?;
            }
            info!(
                files = files.len(),
                "Deleted every lab document's file for the provider purge"
            );
        }
        let purge = self.data.purge_provider_data(provider).await?;
        if self.live.keeps(provider) {
            let athletes = self.live.delete_source_everywhere(provider).await?;
            info!(
                provider,
                athletes, "Deleted every activity store record of the terminated provider"
            );
        }
        Ok(purge)
    }
}
