// ABOUTME: Server-side bridge from pierre_llm::EmbeddingUsageSink to LlmUsageRepository
// ABOUTME: Persists every InstrumentedEmbeddingProvider call into the embedding_usage table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::usage::{EmbeddingUsageRecord, InsertEmbeddingUsage};
use pierre_database::repositories::LlmUsageRepository;
use pierre_llm::embeddings::EmbeddingUsageSink;

/// Adapts [`LlmUsageRepository::insert_embedding_usage`] into the
/// [`EmbeddingUsageSink`] trait so pierre-llm stays free of a direct
/// pierre-database dependency.
pub struct RepositoryEmbeddingSink {
    repo: Arc<dyn LlmUsageRepository>,
}

impl RepositoryEmbeddingSink {
    /// Wrap the shared [`LlmUsageRepository`] for use as an embedding sink.
    #[must_use]
    pub fn new(repo: Arc<dyn LlmUsageRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl EmbeddingUsageSink for RepositoryEmbeddingSink {
    async fn record(&self, params: &InsertEmbeddingUsage<'_>) -> AppResult<EmbeddingUsageRecord> {
        self.repo.insert_embedding_usage(params).await
    }
}
