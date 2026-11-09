// ABOUTME: Hot-reloadable harness config snapshot — compaction + guardrails dispatched from a single registry
// ABOUTME: Loaded from system_settings at startup and re-read by every instance; PUT /admin/settings/harness installs a new snapshot
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Harness Config Registry
//!
//! Holds the canonical [`HarnessConfigDocument`] snapshot the chat pipeline
//! reads through `ServerContext`. Bootstrapped at server startup from
//! `system_settings.harness_config`; if the row is absent or invalid, the
//! registry seeds with [`HarnessConfigDocument::default`] and logs at `error!`
//! for an invalid row.
//!
//! `PUT /admin/settings/harness` validates and persists the new document,
//! then calls [`HarnessConfigRegistry::install`] so subsequent chat turns
//! see the new values without a server restart. That install reaches the one
//! instance the request landed on, so every instance also re-reads the row on
//! an interval ([`HarnessConfigRegistry::reload_from_database`]). A read failure
//! or a row that does not parse keeps the values in force.
//!
//! Each snapshot caches the runtime-shaped projections (`CompactionConfig`,
//! `Arc<TextGuardrails>`) so per-turn reads are an `Arc` clone, not a
//! `Vec<String>` clone.

use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use chrono::{DateTime, Utc};
use pierre_database::backends::factory::Database;
use serde_json::from_str;
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::config::CompactionConfig;

use crate::harness_config_document::{
    validate_document, HarnessCompactionConfig, HarnessConfigDocument, HarnessGuardrailsConfig,
    HarnessMemoryConfig, HarnessVerificationConfig, HARNESS_CONFIG_SETTING_KEY,
};
use crate::text_guardrails::TextGuardrails;

/// Where the current snapshot was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessConfigSource {
    /// Compile-time defaults — used when the `system_settings.harness_config`
    /// row is absent at startup or has not yet been written by an admin.
    Defaults,
    /// Read from `system_settings.harness_config`, at startup or on a poll.
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
    /// Runtime projection consumed by the conversation compactor.
    pub compaction: CompactionConfig,
    /// Runtime projection consumed by [`crate::text_guardrails::TextGuardrails::apply`].
    pub guardrails: Arc<TextGuardrails>,
    /// Verification-stage tunables (`Copy`, read per turn by Stage 17).
    pub verification: HarnessVerificationConfig,
    /// Where the snapshot was sourced from.
    pub source: HarnessConfigSource,
    /// When the snapshot was installed.
    pub loaded_at: DateTime<Utc>,
    /// Why this snapshot does not reflect the stored row, when it cannot: the
    /// row could not be read or parsed.
    pub load_failure: Option<String>,
}

/// What one [`HarnessConfigRegistry::reload_from_database`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessReloadOutcome {
    /// The row matches the snapshot.
    Unchanged,
    /// The snapshot now reflects the row.
    Installed,
    /// The row could not be read; the snapshot stays.
    KeptOnReadFailure,
    /// The row does not parse; the snapshot's values stay, marked.
    KeptOnInvalidRow,
}

/// Thread-safe registry holding the current harness config snapshot.
///
/// Reads (`current`, `current_compaction`, `current_guardrails`) are
/// lock-free and return an `Arc` clone or a `Copy` value, so they are safe
/// to call from the hot chat-turn path. A write is an admin PUT (`install`)
/// or a poll that found the row changed.
#[derive(Debug)]
pub struct HarnessConfigRegistry {
    snapshot: ArcSwap<HarnessConfigSnapshot>,
    instance_nonce: String,
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
    /// values to work with, logs at `error!` and marks the snapshot. The first
    /// poll that reads a good row repairs it.
    pub async fn from_database(db: &Database) -> Self {
        match db.get_system_setting(HARNESS_CONFIG_SETTING_KEY).await {
            Ok(Some(row)) => Self::from_persisted_value(&row.value),
            Ok(None) => {
                info!("harness_config row absent in system_settings; using compiled-in defaults");
                Self::bootstrap()
            }
            Err(e) => {
                error!(
                    error = %e,
                    "harness_config DB load failed; falling back to compiled-in defaults"
                );
                Self::degraded_defaults(
                    "the row could not be read at boot; running the compiled-in defaults".into(),
                )
            }
        }
    }

    /// Parse a JSON document loaded from `system_settings.harness_config`.
    ///
    /// Split out from [`Self::from_database`] so each function stays under
    /// the cognitive-complexity ceiling — the outer arm handles read /
    /// missing / error, this arm handles parse-success / parse-error.
    fn from_persisted_value(json: &str) -> Self {
        match parse_row(json) {
            Ok(doc) => {
                info!(
                    schema_version = doc.schema_version,
                    "harness_config loaded from system_settings"
                );
                Self::new_with(doc, HarnessConfigSource::Database)
            }
            Err(cause) => {
                log_invalid_row(&cause);
                Self::degraded_defaults(format!("{cause}; running the compiled-in defaults"))
            }
        }
    }

    /// A registry on the defaults because the row could not be used at boot.
    fn degraded_defaults(reason: String) -> Self {
        let registry = Self::bootstrap();
        registry.snapshot.store(Arc::new(HarnessConfigSnapshot {
            load_failure: Some(reason),
            ..(*registry.current()).clone()
        }));
        registry
    }

    /// Re-read the row and install it when it differs from the snapshot: one
    /// poll tick.
    ///
    /// A read failure keeps the snapshot and logs at `warn!`. A row that does
    /// not parse, or breaks a rule a save enforces (a hand edit is the only way
    /// to get one), keeps the values in force, marks them and logs at `error!` on
    /// every tick, so an alert stays lit until the row is fixed. Neither path
    /// reverts to defaults; only a read that finds no row does, since that is
    /// what an absent row means at boot too.
    ///
    /// The swap is against the snapshot held before the read, so a tick that
    /// read the row just before an admin PUT landed on this instance cannot put
    /// the older document back.
    pub async fn reload_from_database(&self, db: &Database) -> HarnessReloadOutcome {
        let before = self.snapshot.load_full();
        let row = match db.get_system_setting(HARNESS_CONFIG_SETTING_KEY).await {
            Ok(row) => row,
            Err(e) => {
                warn!(
                    error = %e,
                    "harness_config poll could not read the row; keeping the current values"
                );
                return HarnessReloadOutcome::KeptOnReadFailure;
            }
        };
        let Some(row) = row else {
            return self.settle(
                &before,
                HarnessConfigDocument::default(),
                HarnessConfigSource::Defaults,
            );
        };
        match parse_row(&row.value) {
            Ok(document) => self.settle(&before, document, HarnessConfigSource::Database),
            Err(cause) => {
                log_invalid_row(&cause);
                self.keep_marked_invalid(&before, &cause);
                HarnessReloadOutcome::KeptOnInvalidRow
            }
        }
    }

    /// Install `document` unless the snapshot already holds it, against the
    /// snapshot the caller read before going to the database.
    fn settle(
        &self,
        before: &Arc<HarnessConfigSnapshot>,
        document: HarnessConfigDocument,
        source: HarnessConfigSource,
    ) -> HarnessReloadOutcome {
        if before.load_failure.is_none() && *before.document == document {
            return HarnessReloadOutcome::Unchanged;
        }
        let next = Arc::new(build_snapshot(document, source));
        let prior = self.snapshot.compare_and_swap(before, next);
        if Arc::ptr_eq(&*prior, before) {
            info!("harness_config poll installed a changed row");
            HarnessReloadOutcome::Installed
        } else {
            HarnessReloadOutcome::Unchanged
        }
    }

    /// Say on the snapshot that the row is unusable, keeping its values.
    fn keep_marked_invalid(&self, before: &Arc<HarnessConfigSnapshot>, cause: &str) {
        let reason = format!("{cause}; keeping the last good values");
        if before.load_failure.as_deref() == Some(reason.as_str()) {
            return;
        }
        let next = HarnessConfigSnapshot {
            load_failure: Some(reason),
            ..(**before).clone()
        };
        // Losing this swap to a concurrent install is fine: the next tick marks
        // whatever that install left in place.
        drop(self.snapshot.compare_and_swap(before, Arc::new(next)));
    }

    fn new_with(document: HarnessConfigDocument, source: HarnessConfigSource) -> Self {
        let snapshot = build_snapshot(document, source);
        Self {
            snapshot: ArcSwap::from_pointee(snapshot),
            instance_nonce: Uuid::new_v4().simple().to_string(),
        }
    }

    /// Cheap clone of the entire current snapshot.
    ///
    /// Useful for diagnostic endpoints that want all three views at once.
    #[must_use]
    pub fn current(&self) -> Arc<HarnessConfigSnapshot> {
        self.snapshot.load_full()
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

    /// Memory de-duplication tunables for the current snapshot. Returned by
    /// value because [`HarnessMemoryConfig`] is `Copy`.
    #[must_use]
    pub fn current_memory(&self) -> HarnessMemoryConfig {
        self.read().document.memory
    }

    /// Verification-stage config for the current snapshot. Returned by value
    /// because [`HarnessVerificationConfig`] is `Copy`.
    #[must_use]
    pub fn current_verification(&self) -> HarnessVerificationConfig {
        self.read().verification
    }

    /// A value unique to this registry, so two instances of one revision tell
    /// apart on a screen that asks which one answered.
    #[must_use]
    pub fn instance_nonce(&self) -> &str {
        &self.instance_nonce
    }

    /// Replace the current snapshot with a fresh document.
    ///
    /// Called by the admin PUT handler after
    /// [`crate::harness_config_document::validate_document`] has succeeded
    /// and the row has been persisted. Re-builds the `CompactionConfig`
    /// and `TextGuardrails` projections so subsequent chat turns dispatch
    /// against the new values.
    pub fn install(&self, document: HarnessConfigDocument, source: HarnessConfigSource) {
        self.snapshot
            .store(Arc::new(build_snapshot(document, source)));
    }

    fn read(&self) -> Guard<Arc<HarnessConfigSnapshot>> {
        self.snapshot.load()
    }
}

fn build_snapshot(
    document: HarnessConfigDocument,
    source: HarnessConfigSource,
) -> HarnessConfigSnapshot {
    let compaction = compaction_from(&document.compaction);
    let guardrails = Arc::new(guardrails_from(&document.guardrails));
    let verification = document.verification;
    HarnessConfigSnapshot {
        document: Arc::new(document),
        compaction,
        guardrails,
        verification,
        source,
        loaded_at: Utc::now(),
        load_failure: None,
    }
}

/// The document a stored row holds, or why it cannot be used.
fn parse_row(json: &str) -> Result<HarnessConfigDocument, String> {
    let document: HarnessConfigDocument =
        from_str(json).map_err(|e| format!("the row does not parse ({e})"))?;
    validate_document(&document).map_err(|e| format!("the row is invalid ({e})"))?;
    Ok(document)
}

/// The line an alert matches for a row that does not parse, at boot and on
/// every poll tick: one literal so the two paths cannot drift apart.
fn log_invalid_row(cause: &str) {
    error!(error = %cause, "harness_config row is invalid");
}

fn compaction_from(c: &HarnessCompactionConfig) -> CompactionConfig {
    CompactionConfig {
        window_tokens: c.window_tokens,
        warn_threshold: c.warn_threshold,
        emergency_threshold: c.emergency_threshold,
        summarize_oldest_n: c.summarize_oldest_n as usize,
        sliding_drop_n: c.sliding_drop_n as usize,
        max_messages: c.max_messages as usize,
    }
}

fn guardrails_from(g: &HarnessGuardrailsConfig) -> TextGuardrails {
    TextGuardrails::compile(g.blocked_topics.clone(), g.locales.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The poll's guard against reverting a save: a tick that read the row just
    /// before a PUT installed on this instance loses the swap.
    #[test]
    fn a_tick_holding_a_snapshot_a_save_replaced_cannot_put_the_old_row_back() {
        let registry = HarnessConfigRegistry::bootstrap();
        let held_by_tick = registry.current();

        let mut saved = HarnessConfigDocument::default();
        saved.guardrails.blocked_topics = vec!["saved".to_owned()];
        registry.install(saved.clone(), HarnessConfigSource::AdminUpdate);

        let mut row_the_tick_read = HarnessConfigDocument::default();
        row_the_tick_read.guardrails.blocked_topics = vec!["older".to_owned()];
        assert_eq!(
            registry.settle(
                &held_by_tick,
                row_the_tick_read,
                HarnessConfigSource::Database
            ),
            HarnessReloadOutcome::Unchanged
        );
        assert_eq!(*registry.current().document, saved);
    }
}
