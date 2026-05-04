// ABOUTME: Hot-reloadable dravr-cageux IntelligenceConfig snapshot registry
// ABOUTME: Holds an Arc<IntelligenceConfig<true>> that the platform swaps on contremaitre push
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Cageux Config Registry
//!
//! Holds the canonical [`pierre_intelligence::IntelligenceConfig`] snapshot
//! that the rest of the server reads through `ServerContext`. Lives
//! outside `contremaitre/` so it is always compiled — `server-mcp-stdio`
//! and `server-mcp-bridge` feature sets disable the contremaitre hot-reload
//! module but still need the cageux config snapshot for every analyzer,
//! engine, and handler.
//!
//! The registry starts seeded from `IntelligenceConfig::load()` (compiled-in
//! defaults overlaid with `INTELLIGENCE_*` env vars) so the server boots
//! correctly even when contremaitre is unreachable or disabled at compile
//! time. When the `contremaitre` feature is on, `contremaitre::sync` fetches
//! `config/cageux.yaml` from the manifest and calls
//! [`CageuxConfigRegistry::apply_overlay`] to apply the layered stack
//! (`defaults → env → YAML overlay → validate`) before installing the
//! resulting snapshot. Webhook push events run the same path again without
//! restarting the process.

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_intelligence::IntelligenceConfig;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::errors::{AppError, AppResult};

/// Where the current snapshot was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CageuxConfigSource {
    /// Seeded from `IntelligenceConfig::load()` — compiled-in defaults plus
    /// `INTELLIGENCE_*` environment variables. The server always has a
    /// bootstrap snapshot even when contremaitre is unreachable.
    Bootstrap,
    /// Installed by the contremaitre sync after applying a YAML overlay
    /// from `config/cageux.yaml`.
    Contremaitre,
}

/// Current snapshot plus metadata about where it came from.
#[derive(Debug, Clone)]
pub struct CageuxConfigSnapshot {
    /// The live configuration. Cloning the `Arc` is cheap; callers should
    /// hold onto their clone for the duration of a request rather than
    /// re-reading the registry repeatedly.
    pub config: Arc<IntelligenceConfig<true>>,
    /// SHA-256 hex digest of the overlay YAML that produced this snapshot,
    /// or `None` when the current snapshot is the bootstrap.
    pub overlay_sha256: Option<String>,
    /// Where the current snapshot came from.
    pub source: CageuxConfigSource,
    /// When the current snapshot was installed.
    pub loaded_at: DateTime<Utc>,
}

/// Thread-safe registry holding one hot-swappable cageux intelligence config.
///
/// Initialized with a `Bootstrap`-sourced snapshot built from
/// [`IntelligenceConfig::load`]. The contremaitre sync (when compiled in)
/// replaces it with a `Contremaitre`-sourced snapshot once the overlay YAML
/// has been fetched and validated.
pub struct CageuxConfigRegistry {
    snapshot: RwLock<CageuxConfigSnapshot>,
}

impl CageuxConfigRegistry {
    /// Create a registry seeded with the compiled-in defaults overlaid with
    /// `INTELLIGENCE_*` environment variables.
    ///
    /// Infallible: if `IntelligenceConfig::load()` fails (bad env
    /// override), the registry falls back to the compiled-in defaults and
    /// logs a warning. The server startup pipeline is expected to have
    /// already called `init_configs()` / `init_all_configs()` which fails
    /// fast on invalid env values, so reaching the fallback path here
    /// indicates either a programming error or a transient env mutation
    /// between the two calls.
    #[must_use]
    pub fn from_env() -> Self {
        match IntelligenceConfig::load() {
            Ok(config) => Self::new_with(config, None, CageuxConfigSource::Bootstrap),
            Err(e) => {
                warn!(
                    error = %e,
                    "cageux IntelligenceConfig::load() failed at registry init, \
                     falling back to compiled-in defaults"
                );
                Self::new_with(
                    IntelligenceConfig::default(),
                    None,
                    CageuxConfigSource::Bootstrap,
                )
            }
        }
    }

    /// Internal constructor sharing snapshot-building logic between the
    /// bootstrap and overlay-applying paths.
    fn new_with(
        config: IntelligenceConfig<true>,
        overlay_sha256: Option<String>,
        source: CageuxConfigSource,
    ) -> Self {
        Self {
            snapshot: RwLock::new(CageuxConfigSnapshot {
                config: Arc::new(config),
                overlay_sha256,
                source,
                loaded_at: Utc::now(),
            }),
        }
    }

    /// Return a cheap clone of the current snapshot's `Arc`.
    ///
    /// Callers that need to hold onto the config across await points or
    /// pass it to cageux engine constructors should clone the returned
    /// `Arc` and use the clone for the rest of the request.
    #[must_use]
    pub fn current(&self) -> Arc<IntelligenceConfig<true>> {
        Arc::clone(&self.read().config)
    }

    /// SHA-256 of the overlay YAML that produced the current snapshot.
    ///
    /// Returns `None` when the current snapshot is the env-only bootstrap.
    /// Used by the contremaitre sync engine to short-circuit when the
    /// manifest entry's hash matches the installed snapshot.
    #[must_use]
    pub fn current_overlay_sha256(&self) -> Option<String> {
        self.read().overlay_sha256.clone()
    }

    /// Metadata snapshot for diagnostics / admin endpoints.
    #[must_use]
    pub fn snapshot(&self) -> CageuxConfigSnapshot {
        self.read().clone()
    }

    /// Apply a YAML overlay on top of the layered stack and install the
    /// resulting validated snapshot.
    ///
    /// Runs the same pipeline as [`IntelligenceConfig::with_overlay`]:
    /// compiled-in defaults → `INTELLIGENCE_*` env vars → this YAML overlay
    /// → `validate()`. A validation failure leaves the previously installed
    /// snapshot untouched.
    ///
    /// # Errors
    ///
    /// Returns an error if the overlay fails to parse or validate. In
    /// either case the previous snapshot remains live.
    pub fn apply_overlay(&self, overlay_yaml: &str) -> AppResult<()> {
        let overlay_sha = compute_sha256(overlay_yaml.as_bytes());
        let config = IntelligenceConfig::with_overlay(overlay_yaml)
            .map_err(|e| AppError::internal(format!("cageux overlay: {e}")))?;
        self.install(config, Some(overlay_sha), CageuxConfigSource::Contremaitre);
        Ok(())
    }

    /// Replace the current snapshot with the given config.
    ///
    /// Lower-level than [`Self::apply_overlay`]; intended for tests and for
    /// admin write-back paths that have already produced a validated
    /// config. Production sync should go through `apply_overlay`.
    pub fn install(
        &self,
        config: IntelligenceConfig<true>,
        overlay_sha256: Option<String>,
        source: CageuxConfigSource,
    ) {
        let mut guard = self.write();
        *guard = CageuxConfigSnapshot {
            config: Arc::new(config),
            overlay_sha256,
            source,
            loaded_at: Utc::now(),
        };
    }

    fn read(&self) -> RwLockReadGuard<'_, CageuxConfigSnapshot> {
        self.snapshot.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, CageuxConfigSnapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Compute a SHA-256 hex digest for an overlay body.
///
/// Local helper so this module does not depend on the contremaitre feature
/// gate. Mirrors `contremaitre::manifest::compute_sha256`.
fn compute_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}
