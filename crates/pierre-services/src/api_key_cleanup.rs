// ABOUTME: Background worker deactivating expired API keys, hourly, on the shared spawn_periodic loop
// ABOUTME: One cross-instance DB sweep: the worker ledger runs each due pass once across instances
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Expired API key sweep
//!
//! Marks every key past its `expires_at` inactive, so the key listings and the
//! admin views show it as the dead credential it is.

use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::AppResult;
use pierre_database::backends::ApiKeyRepository;
use pierre_database::repositories::WorkerRunRepository;
use tokio::task::AbortHandle;
use tracing::info;

use crate::periodic::spawn_periodic;

/// Ledger name of the worker.
const WORKER_NAME: &str = "api-key cleanup";

/// One pass an hour.
const PERIOD: Duration = Duration::from_hours(1);

/// Start the hourly expired-API-key sweep.
///
/// Returns the worker's abort handle; callers discard it, as a restart
/// re-arms the worker from the ledger.
pub fn start_api_key_cleanup_task(
    api_keys: Arc<dyn ApiKeyRepository>,
    ledger: Arc<dyn WorkerRunRepository>,
) -> AbortHandle {
    spawn_periodic(WORKER_NAME, PERIOD, ledger, move || {
        let api_keys = Arc::clone(&api_keys);
        async move { sweep_expired_api_keys(api_keys.as_ref()).await.map(|_| ()) }
    })
}

/// Deactivate every expired API key that is still active, returning how many.
///
/// # Errors
///
/// Returns the repository error when the delete fails; `spawn_periodic` logs
/// it and retries the pass.
pub async fn sweep_expired_api_keys(api_keys: &dyn ApiKeyRepository) -> AppResult<u64> {
    let deleted = api_keys.cleanup_expired().await?;
    if deleted > 0 {
        info!(
            worker = WORKER_NAME,
            deactivated = deleted,
            "deactivated expired API keys"
        );
    }
    Ok(deleted)
}
