// ABOUTME: Pluggable embedding provider abstraction for the coaching harness memory pipeline
// ABOUTME: Canonical trait + Gemini implementation; other backends (Groq, OpenAI) land in later tiers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Embeddings
//!
//! Small, focused abstraction for turning text into a dense float vector.
//! The coaching harness uses embeddings to vector-search over user facts,
//! coach notes, and compacted turn summaries.
//!
//! The trait intentionally has a single method — `embed` — and returns a
//! `Vec<f32>` so both `SQLite` (BLOB-encoded little-endian f32 sequences) and
//! Postgres (BYTEA with the same format, upgradable to `vector(N)` when
//! pgvector is available) can persist embeddings without introducing a new
//! cross-crate type.

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::llm_client;
use pierre_core::models::usage::{EmbeddingUsageRecord, InsertEmbeddingUsage};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Number of dimensions in an embedding vector.
pub type EmbeddingDim = usize;

/// Pluggable embedding provider for the harness memory pipeline.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Stable identifier used in cost tracking and admin UI.
    fn name(&self) -> &'static str;

    /// Default model identifier reported in usage rows.
    fn model(&self) -> &str;

    /// Vector dimensionality — callers use this to pre-size storage and to
    /// refuse to mix embeddings from providers with different widths.
    fn dimensions(&self) -> EmbeddingDim;

    /// Turn the input text into a single embedding vector.
    async fn embed(&self, text: &str) -> AppResult<Vec<f32>>;
}

/// Writer adapter for the `embedding_usage` table.
///
/// Decouples [`EmbeddingProvider`] from the full
/// [`pierre_database`] dependency graph: pierre-llm does not depend on
/// pierre-database directly, so the server wires an implementation of
/// this trait that bridges to `LlmUsageRepository::insert_embedding_usage`.
#[async_trait]
pub trait EmbeddingUsageSink: Send + Sync {
    /// Persist a single embedding call.
    async fn record(&self, params: &InsertEmbeddingUsage<'_>) -> AppResult<EmbeddingUsageRecord>;
}

/// USD cost per 1000 characters of input, used as the fallback pricing
/// for embedding calls. Matches Gemini's `text-embedding-004` posted rate
/// of $0.025 per 1M input tokens (~$0.00625 per 1K characters at 4 chars/token).
const EMBEDDING_COST_PER_1K_CHARS: f64 = 0.000_006_25;

/// Instrumentation wrapper around a raw [`EmbeddingProvider`].
///
/// Records every call in `embedding_usage` via the supplied
/// [`EmbeddingUsageSink`]. Tenant + user context must be set per-call
/// via [`Self::embed_for`] — callers that do not carry that context
/// (e.g. startup seeders) should use the inner provider directly.
pub struct InstrumentedEmbeddingProvider {
    inner: Box<dyn EmbeddingProvider>,
    sink: Arc<dyn EmbeddingUsageSink>,
}

impl InstrumentedEmbeddingProvider {
    /// Wrap `inner` so every embed call is written to `embedding_usage`
    /// via `sink`.
    #[must_use]
    pub fn new(inner: Box<dyn EmbeddingProvider>, sink: Arc<dyn EmbeddingUsageSink>) -> Self {
        Self { inner, sink }
    }

    /// Execute an embedding call and record it against `(tenant_id, user_id)`.
    /// Recording failures do not fail the embed — the warning is logged
    /// so embedding-driven features (memory, harness) never degrade due
    /// to telemetry issues.
    ///
    /// # Errors
    ///
    /// Returns whatever the inner provider's `embed` call returns when the
    /// embedding HTTP request fails; sink-write errors are swallowed and
    /// only logged.
    pub async fn embed_for(
        &self,
        tenant_id: &str,
        user_id: &str,
        text: &str,
    ) -> AppResult<Vec<f32>> {
        let vector = self.inner.embed(text).await?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let input_tokens = i64::try_from(text.chars().count().div_ceil(4)).unwrap_or(i64::MAX);
        let cost_usd = (text.chars().count() as f64) / 1000.0 * EMBEDDING_COST_PER_1K_CHARS;
        let params = InsertEmbeddingUsage {
            tenant_id,
            user_id,
            provider: self.inner.name(),
            model: self.inner.model(),
            input_tokens,
            cost_usd,
        };
        if let Err(e) = self.sink.record(&params).await {
            tracing::warn!(tenant_id, user_id, "Failed to record embedding usage: {e}");
        }
        Ok(vector)
    }

    /// Borrow the wrapped provider to query `name()` / `model()` /
    /// `dimensions()` without exposing the raw embed path.
    #[must_use]
    pub fn inner(&self) -> &dyn EmbeddingProvider {
        self.inner.as_ref()
    }
}

// ============================================================================
// Gemini implementation
// ============================================================================

/// Gemini text embedding provider.
///
/// Uses Google's `text-embedding-004` model by default, which returns 768-dim
/// vectors. The API key is passed at construction time. The provider reuses
/// the shared Pierre LLM HTTP client so all outbound LLM traffic goes through
/// the same pool and timeout settings.
pub struct GeminiEmbeddingProvider {
    api_key: String,
    model: String,
    dimensions: EmbeddingDim,
}

impl GeminiEmbeddingProvider {
    /// Default Gemini embedding model.
    pub const DEFAULT_MODEL: &'static str = "text-embedding-004";

    /// Output dimensionality of [`Self::DEFAULT_MODEL`].
    pub const DEFAULT_DIMENSIONS: EmbeddingDim = 768;

    /// Build a provider with the default model and dimensionality.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: Self::DEFAULT_MODEL.to_owned(),
            dimensions: Self::DEFAULT_DIMENSIONS,
        }
    }

    /// Override the model name. Does not change the advertised dimension —
    /// callers who swap models must use [`Self::with_dimensions`] as well.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the advertised dimensionality.
    #[must_use]
    pub const fn with_dimensions(mut self, dimensions: EmbeddingDim) -> Self {
        self.dimensions = dimensions;
        self
    }
}

#[derive(Serialize)]
struct GeminiEmbedRequest<'a> {
    model: String,
    content: GeminiEmbedContent<'a>,
}

#[derive(Serialize)]
struct GeminiEmbedContent<'a> {
    parts: [GeminiEmbedPart<'a>; 1],
}

#[derive(Serialize)]
struct GeminiEmbedPart<'a> {
    text: &'a str,
}

#[derive(Deserialize)]
struct GeminiEmbedResponse {
    embedding: GeminiEmbedding,
}

#[derive(Deserialize)]
struct GeminiEmbedding {
    values: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for GeminiEmbeddingProvider {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> EmbeddingDim {
        self.dimensions
    }

    async fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        if text.is_empty() {
            return Err(AppError::invalid_input("embedding input text is empty"));
        }

        let client = llm_client();
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:embedContent?key={key}",
            model = self.model,
            key = self.api_key
        );

        let body = GeminiEmbedRequest {
            model: format!("models/{}", self.model),
            content: GeminiEmbedContent {
                parts: [GeminiEmbedPart { text }],
            },
        };

        let resp = client.post(&url).json(&body).send().await.map_err(|e| {
            AppError::external_service("gemini", format!("embed request failed: {e}"))
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AppError::external_service(
                "gemini",
                format!("embed request returned {status}: {body_text}"),
            ));
        }

        let parsed: GeminiEmbedResponse = resp.json().await.map_err(|e| {
            AppError::external_service("gemini", format!("embed decode failed: {e}"))
        })?;

        Ok(parsed.embedding.values)
    }
}

#[cfg(test)]
mod tests {
    use super::{EmbeddingProvider, GeminiEmbeddingProvider};

    #[test]
    fn default_dimensions_match_advertised_constant() {
        let provider = GeminiEmbeddingProvider::new("test-key");
        assert_eq!(
            provider.dimensions(),
            GeminiEmbeddingProvider::DEFAULT_DIMENSIONS
        );
        assert_eq!(provider.name(), "gemini");
    }

    #[test]
    fn with_dimensions_overrides_default() {
        let provider = GeminiEmbeddingProvider::new("k").with_dimensions(256);
        assert_eq!(provider.dimensions(), 256);
    }
}
