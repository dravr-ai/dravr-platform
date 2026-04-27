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
use serde::{Deserialize, Serialize};

/// Number of dimensions in an embedding vector.
pub type EmbeddingDim = usize;

/// Pluggable embedding provider for the harness memory pipeline.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Stable identifier used in cost tracking and admin UI.
    fn name(&self) -> &'static str;

    /// Vector dimensionality — callers use this to pre-size storage and to
    /// refuse to mix embeddings from providers with different widths.
    fn dimensions(&self) -> EmbeddingDim;

    /// Turn the input text into a single embedding vector.
    async fn embed(&self, text: &str) -> AppResult<Vec<f32>>;
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
