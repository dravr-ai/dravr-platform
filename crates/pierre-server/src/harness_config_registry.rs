// ABOUTME: Hot-reloadable harness config snapshot — compaction + guardrails dispatched from a single registry
// ABOUTME: Loaded from system_settings at startup; PUT /admin/settings/harness installs a new snapshot
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Harness Config Registry
//!
//! Holds the canonical [`HarnessConfigDocument`] snapshot the chat pipeline
//! reads through `ServerContext`. Bootstrapped at server startup from
//! `system_settings.harness_config`; if the row is absent or invalid, the
//! registry seeds with [`HarnessConfigDocument::default`] and logs a warning.
//!
//! `PUT /admin/settings/harness` validates and persists the new document,
//! then calls [`HarnessConfigRegistry::install`] so subsequent chat turns
//! see the new values without a server restart.
//!
//! Each snapshot caches the runtime-shaped projections (`CompactionConfig`,
//! `Arc<TextGuardrails>`) so per-turn reads are an `Arc` clone, not a
//! `Vec<String>` clone.

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_database::backends::factory::Database;
use serde_json::from_str;
use tracing::{info, warn};

use crate::config::text_guardrails::TextGuardrails;
use crate::routes::admin::harness_config::{
    HarnessCompactionConfig, HarnessConfigDocument, HarnessGuardrailsConfig,
    HARNESS_CONFIG_SETTING_KEY,
};
use crate::services::conversation_compaction::CompactionConfig;

/// Where the current snapshot was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessConfigSource {
    /// Compile-time defaults — used when the `system_settings.harness_config`
    /// row is absent at startup or has not yet been written by an admin.
    Defaults,
    /// Loaded from `system_settings.harness_config` at startup.
    Database,
    /// Installed by the admin PUT handler after a successful write.
    AdminUpdate,
}

/// Snapshot held by the registry plus pre-built runtime projections.
///
/// `compaction` is `Copy`, so reads return it by value. `guardrails` is
/// wrapped in `Arc` so reads return a cheap atomic clone without
/// re-allocating the topic / trigger / disclaimer strings.
#[derive(Debug, Clone)]
pub struct HarnessConfigSnapshot {
    /// Source document as persisted in `system_settings.harness_config`.
    pub document: Arc<HarnessConfigDocument>,
    /// Runtime projection consumed by [`crate::services::conversation_compaction::ConversationCompactor`].
    pub compaction: CompactionConfig,
    /// Runtime projection consumed by [`crate::config::text_guardrails::TextGuardrails::apply`].
    pub guardrails: Arc<TextGuardrails>,
    /// Where the snapshot was sourced from.
    pub source: HarnessConfigSource,
    /// When the snapshot was installed.
    pub loaded_at: DateTime<Utc>,
}

/// Thread-safe registry holding the current harness config snapshot.
///
/// Reads (`current`, `current_compaction`, `current_guardrails`) take a
/// short read-lock and return an `Arc` clone or a `Copy` value, so they
/// are safe to call from the hot chat-turn path. Writes (`install`) are
/// expected only on admin PUT — a couple per day at most.
pub struct HarnessConfigRegistry {
    snapshot: RwLock<HarnessConfigSnapshot>,
}

impl HarnessConfigRegistry {
    /// Build a registry seeded with compile-time defaults.
    ///
    /// Used when the `system_settings.harness_config` row is absent at
    /// startup (fresh database) or when the row fails to deserialize.
    #[must_use]
    pub fn bootstrap() -> Self {
        Self::new_with(
            HarnessConfigDocument::default(),
            HarnessConfigSource::Defaults,
        )
    }

    /// Load the document from `system_settings.harness_config` at startup.
    ///
    /// On any read or parse failure, falls back to compile-time defaults
    /// so the server still boots and the chat pipeline still has runtime
    /// values to work with. Each fallback path emits a `warn!` with the
    /// underlying error so operators can spot a corrupted row.
    pub async fn from_database(db: &Database) -> Self {
        match db.get_system_setting(HARNESS_CONFIG_SETTING_KEY).await {
            Ok(Some(row)) => Self::from_persisted_value(&row.value),
            Ok(None) => {
                info!("harness_config row absent in system_settings; using compiled-in defaults");
                Self::bootstrap()
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "harness_config DB load failed; falling back to compiled-in defaults"
                );
                Self::bootstrap()
            }
        }
    }

    /// Parse a JSON document loaded from `system_settings.harness_config`.
    ///
    /// Split out from [`Self::from_database`] so each function stays under
    /// the cognitive-complexity ceiling — the outer arm handles read /
    /// missing / error, this arm handles parse-success / parse-error.
    fn from_persisted_value(json: &str) -> Self {
        match from_str::<HarnessConfigDocument>(json) {
            Ok(doc) => {
                info!(
                    schema_version = doc.schema_version,
                    "harness_config loaded from system_settings"
                );
                Self::new_with(doc, HarnessConfigSource::Database)
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "harness_config row failed to deserialize; falling back to compiled-in defaults"
                );
                Self::bootstrap()
            }
        }
    }

    fn new_with(document: HarnessConfigDocument, source: HarnessConfigSource) -> Self {
        let snapshot = build_snapshot(document, source);
        Self {
            snapshot: RwLock::new(snapshot),
        }
    }

    /// Cheap clone of the entire current snapshot.
    ///
    /// Useful for diagnostic endpoints that want all three views at once.
    #[must_use]
    pub fn current(&self) -> HarnessConfigSnapshot {
        self.read().clone()
    }

    /// Compaction config for the current snapshot. Returned by value
    /// because [`CompactionConfig`] is `Copy`.
    #[must_use]
    pub fn current_compaction(&self) -> CompactionConfig {
        self.read().compaction
    }

    /// Guardrails view for the current snapshot. Cheap atomic `Arc` clone.
    #[must_use]
    pub fn current_guardrails(&self) -> Arc<TextGuardrails> {
        Arc::clone(&self.read().guardrails)
    }

    /// Replace the current snapshot with a fresh document.
    ///
    /// Called by the admin PUT handler after [`crate::routes::admin::harness_config::validate_document`]
    /// has succeeded and the row has been persisted. Re-builds the
    /// `CompactionConfig` and `TextGuardrails` projections so subsequent
    /// chat turns dispatch against the new values.
    pub fn install(&self, document: HarnessConfigDocument, source: HarnessConfigSource) {
        let next = build_snapshot(document, source);
        let mut guard = self.write();
        *guard = next;
    }

    fn read(&self) -> RwLockReadGuard<'_, HarnessConfigSnapshot> {
        self.snapshot.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HarnessConfigSnapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn build_snapshot(
    document: HarnessConfigDocument,
    source: HarnessConfigSource,
) -> HarnessConfigSnapshot {
    let compaction = compaction_from(&document.compaction);
    let guardrails = Arc::new(guardrails_from(&document.guardrails));
    HarnessConfigSnapshot {
        document: Arc::new(document),
        compaction,
        guardrails,
        source,
        loaded_at: Utc::now(),
    }
}

fn compaction_from(c: &HarnessCompactionConfig) -> CompactionConfig {
    CompactionConfig {
        window_tokens: c.window_tokens,
        warn_threshold: c.warn_threshold,
        emergency_threshold: c.emergency_threshold,
        summarize_oldest_n: c.summarize_oldest_n as usize,
        sliding_drop_n: c.sliding_drop_n as usize,
    }
}

fn guardrails_from(g: &HarnessGuardrailsConfig) -> TextGuardrails {
    TextGuardrails {
        max_response_chars: g.max_response_chars as usize,
        blocked_topics: g.blocked_topics.clone(),
        disclaimer_triggers: g.disclaimer_triggers.clone(),
        disclaimer_text: g.disclaimer_text.clone(),
    }
}
