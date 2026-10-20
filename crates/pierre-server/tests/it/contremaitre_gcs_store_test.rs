// ABOUTME: Integration tests for the GCS-backed PromptStore implementation
// ABOUTME: Uses a stub TokenProvider so tests don't need a real metadata server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests covering the GCS-backed
//! [`pierre_contremaitre::GcsPromptStore`].
//!
//! Production reads obtain an `OAuth2` token from the GCP metadata server,
//! which is unreachable from `cargo test`. These tests inject a stub
//! [`pierre_core::gcp_token::TokenProvider`] so the store can be
//! constructed and exercised without leaving the test binary.

use std::sync::Arc;

use async_trait::async_trait;

use pierre_contremaitre::{GcsPromptStore, PromptStore};
use pierre_core::errors::AppResult;
use pierre_core::gcp_token::TokenProvider;

/// Stub provider returning a fixed token. Avoids depending on the GCP
/// metadata server, which is unreachable from `cargo test`.
struct StaticTokenProvider {
    token: String,
}

#[async_trait]
impl TokenProvider for StaticTokenProvider {
    async fn access_token(&self) -> AppResult<String> {
        Ok(self.token.clone())
    }
}

#[test]
fn backend_label_is_gcs() {
    let store = GcsPromptStore::with_token_provider(
        "test-bucket".to_owned(),
        Arc::new(StaticTokenProvider {
            token: "fake-access-token".to_owned(),
        }),
    );
    assert_eq!(store.backend_label(), "gcs");
}
