// ABOUTME: Extension context for dependency injection of MCP protocol services
// ABOUTME: Contains sampling peer and progress notification channels for MCP extensions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::mcp::sampling_peer::SamplingPeer;
use crate::mcp::schema::ProgressNotification;
use crate::protocols::universal::types::CancellationToken;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Extension context containing protocol-extension dependencies
///
/// Provides MCP-extension dependencies for server-initiated LLM requests
/// and progress tracking.
///
/// # Dependencies
/// - `sampling_peer`: Optional sampling peer for server-initiated LLM requests (stdio transport only)
/// - `progress_notification_sender`: Optional channel for progress notifications (stdio transport only)
/// - `cancellation_registry`: Registry mapping progress tokens to cancellation tokens
#[derive(Clone)]
pub struct ExtensionContext {
    sampling_peer: Option<Arc<SamplingPeer>>,
    progress_notification_sender: Option<mpsc::UnboundedSender<ProgressNotification>>,
    cancellation_registry: Arc<RwLock<HashMap<String, CancellationToken>>>,
}

impl ExtensionContext {
    /// Create new extension context
    #[must_use]
    pub const fn new(
        sampling_peer: Option<Arc<SamplingPeer>>,
        progress_notification_sender: Option<mpsc::UnboundedSender<ProgressNotification>>,
        cancellation_registry: Arc<RwLock<HashMap<String, CancellationToken>>>,
    ) -> Self {
        Self {
            sampling_peer,
            progress_notification_sender,
            cancellation_registry,
        }
    }

    /// Get sampling peer for server-initiated LLM requests
    #[must_use]
    pub const fn sampling_peer(&self) -> &Option<Arc<SamplingPeer>> {
        &self.sampling_peer
    }

    /// Get progress notification sender for progress updates
    #[must_use]
    pub const fn progress_notification_sender(
        &self,
    ) -> &Option<mpsc::UnboundedSender<ProgressNotification>> {
        &self.progress_notification_sender
    }

    /// Get cancellation registry for progress token -> cancellation token mapping
    #[must_use]
    pub const fn cancellation_registry(&self) -> &Arc<RwLock<HashMap<String, CancellationToken>>> {
        &self.cancellation_registry
    }

    /// Register a cancellation token for a progress token
    pub async fn register_cancellation_token(
        &self,
        progress_token: String,
        cancellation_token: CancellationToken,
    ) {
        let mut registry = self.cancellation_registry.write().await;
        registry.insert(progress_token, cancellation_token);
    }

    /// Cancel an operation by progress token
    pub async fn cancel_by_progress_token(&self, progress_token: &str) {
        let registry = self.cancellation_registry.read().await;
        if let Some(token) = registry.get(progress_token) {
            token.cancel().await;
        }
    }

    /// Cleanup a cancellation token after operation completes
    pub async fn cleanup_cancellation_token(&self, progress_token: &str) {
        let mut registry = self.cancellation_registry.write().await;
        registry.remove(progress_token);
    }
}
