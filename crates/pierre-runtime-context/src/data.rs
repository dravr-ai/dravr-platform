// ABOUTME: Data context for dependency injection of database, cache, and provider services
// ABOUTME: Contains database, cache, provider registry, and intelligence services for data operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_cache::Cache;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::ProviderPurge;
use pierre_database::RepositoryRegistry;
use pierre_providers::ProviderRegistry;
use std::sync::Arc;

/// Data context containing database, cache, and provider dependencies
///
/// This context provides all data-related dependencies needed for
/// database operations, caching, provider integration, and activity intelligence.
///
/// # Dependencies
/// - `database`: Lifecycle and system settings only
/// - `repos`: Primary data access through trait-object registry
/// - `cache`: Cache layer for performance optimization
/// - `provider_registry`: Registry of external service providers
/// - `provider_purge`: the provider data purge, in a context that purges
#[derive(Clone, Debug)]
pub struct DataContext {
    database: Arc<Database>,
    repos: Arc<RepositoryRegistry>,
    cache: Arc<Cache>,
    provider_registry: Arc<ProviderRegistry>,
    provider_purge: Option<Arc<dyn ProviderPurge>>,
}

impl DataContext {
    /// Create new data context
    #[must_use]
    pub fn new(
        database: Arc<Database>,
        repos: Arc<RepositoryRegistry>,
        cache: Arc<Cache>,
        provider_registry: Arc<ProviderRegistry>,
    ) -> Self {
        Self {
            database,
            repos,
            cache,
            provider_registry,
            provider_purge: None,
        }
    }

    /// The same context, purging a provider's data through `purge`: a
    /// server's, whose disconnects delete what a provider contributed.
    #[must_use]
    pub fn with_provider_purge(mut self, purge: Arc<dyn ProviderPurge>) -> Self {
        self.provider_purge = Some(purge);
        self
    }

    /// The provider data purge this context deletes a provider's data
    /// through.
    ///
    /// # Errors
    ///
    /// Returns an internal error in a context built without one (a CLI
    /// minting a session): it holds no activity store, so a purge there
    /// would leave the store's records behind.
    pub fn provider_purge(&self) -> AppResult<&Arc<dyn ProviderPurge>> {
        self.provider_purge.as_ref().ok_or_else(|| {
            AppError::internal("This data context was built without the provider data purge")
        })
    }

    /// Get database for lifecycle and system settings
    #[must_use]
    pub fn database(&self) -> &Arc<Database> {
        &self.database
    }

    /// Get repository registry for data access
    #[must_use]
    pub fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.repos
    }

    /// Get cache for performance optimization
    #[must_use]
    pub const fn cache(&self) -> &Arc<Cache> {
        &self.cache
    }

    /// Get provider registry for external integrations
    #[must_use]
    pub const fn provider_registry(&self) -> &Arc<ProviderRegistry> {
        &self.provider_registry
    }
}
