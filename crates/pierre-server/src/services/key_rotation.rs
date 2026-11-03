// ABOUTME: Starts key rotation in the server: the per-instance keyring reload and the fleet-wide rotation controller
// ABOUTME: Registers barrage's export links as an external ciphertext holder the controller waits on before disabling a key

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Key rotation in the server (carnet#1347)
//!
//! - **Reload**, on every instance, every [`RELOAD_PERIOD`]: re-read the
//!   keyset and rebuild this process's keyring when it changed
//!   ([`KeysetLoader::refresh`]). Once the keyset is adopted, a process that
//!   cannot reload refuses to seal after the fleet window.
//! - **Controller**, once across the fleet through the worker ledger, every
//!   [`CONTROLLER_PERIOD`]: promote, sweep, disable, schedule
//!   ([`KeysetController::tick`]). It does nothing before adoption.
//!
//! Barrage stores each import's export link sealed by the platform, in its
//! own `import_jobs` table, where the sweep cannot re-seal it. The link is
//! counted instead ([`ArchiveLinkHolder`]): a data key is not disabled while
//! a job created before its demotion (plus the fleet window) holds a link.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dravr_barrage::store::ImportJobStore;
use pierre_auth::key_management::KeyManager;
use pierre_auth::keyset::{
    CiphertextHolder, KeysetController, KeysetLoader, CONTROLLER_PERIOD, CONTROLLER_WORKER,
    RELOAD_PERIOD,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::shared::key_material::KEYSET_RELOAD_FAILED;
use pierre_services::periodic::spawn_periodic;
use pierre_services::supervised::spawn_supervised_loop;
use tracing::error;

use crate::mcp::resources::ServerContext;

/// The name the reload loop's log lines carry.
pub const RELOAD_LOOP: &str = "keyset reload";

/// Barrage's import jobs, as the controller counts the export links they
/// still hold.
pub struct ArchiveLinkHolder {
    jobs: Arc<dyn ImportJobStore>,
}

impl fmt::Debug for ArchiveLinkHolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArchiveLinkHolder").finish_non_exhaustive()
    }
}

impl ArchiveLinkHolder {
    /// The holder over barrage's job store.
    #[must_use]
    pub const fn new(jobs: Arc<dyn ImportJobStore>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl CiphertextHolder for ArchiveLinkHolder {
    fn name(&self) -> &'static str {
        "archive import links"
    }

    async fn count_sealed_before(&self, before: DateTime<Utc>) -> AppResult<u64> {
        self.jobs
            .count_jobs_holding_link_created_before(before)
            .await
            .map_err(|e| AppError::internal(format!("counting archive import links: {e}")))
    }
}

/// The external holders this deployment registers: barrage's job store when
/// imports are wired here, or, on Postgres where imports did not wire, the
/// same `barrage` schema over the platform pool, so jobs another instance
/// created are still counted. A count on a schema that does not exist
/// fails, and a failed count holds the key.
fn holders(resources: &Arc<ServerContext>) -> Vec<Arc<dyn CiphertextHolder>> {
    if let Ok(wiring) = resources.common.archive_imports.wiring() {
        return vec![Arc::new(ArchiveLinkHolder::new(Arc::clone(
            &wiring.stores.jobs,
        )))];
    }
    #[cfg(feature = "postgresql")]
    if let Some(pool) = resources.agent.database.postgres_pool() {
        use dravr_barrage::store::sql::postgres::PostgresJobStore;
        return vec![Arc::new(ArchiveLinkHolder::new(Arc::new(
            PostgresJobStore::new(pool.clone()),
        )))];
    }
    Vec::new()
}

/// Start the keyring reload on this instance and the rotation controller on
/// the fleet's ledger.
///
/// A KEK provider that cannot be built is logged under
/// `key_material.keyset_reload_failed`, which pages: without a reload, an
/// adopted keyset stops sealing after the fleet window.
pub fn start_key_rotation(resources: &Arc<ServerContext>) {
    let kek = match KeyManager::configured_kek_provider() {
        Ok(kek) => Arc::from(kek),
        Err(e) => {
            error!(
                event = KEYSET_RELOAD_FAILED,
                error = %e,
                "the KEK provider could not be built; the keyring will not reload"
            );
            return;
        }
    };
    let database = Arc::clone(&resources.agent.database);
    let loader = Arc::new(KeysetLoader::new(
        Arc::clone(&kek),
        Arc::clone(database.keyring()),
    ));

    let reloading = (Arc::clone(&database), Arc::clone(&loader));
    spawn_supervised_loop(RELOAD_LOOP, RELOAD_PERIOD, move || {
        let (database, loader) = reloading.clone();
        async move {
            loader
                .refresh_or_report(database.as_security_repository())
                .await;
        }
    });

    let controller = Arc::new(KeysetController::new(
        loader,
        kek,
        holders(resources),
        resources.common.config.security.key_rotation_days,
    ));
    spawn_periodic(
        CONTROLLER_WORKER,
        CONTROLLER_PERIOD,
        Arc::clone(&resources.common.repos.worker_runs),
        move || {
            let (database, controller) = (Arc::clone(&database), Arc::clone(&controller));
            async move {
                controller
                    .tick(database.as_security_repository(), Utc::now())
                    .await
                    .map(|_| ())
            }
        },
    );
}
