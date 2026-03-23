// ABOUTME: Bridge crate re-exporting dravr-commere push notification service
// ABOUTME: Provides error conversion between CommereError and AppError
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Notifications (Bridge)
//!
//! Thin compatibility layer re-exporting from `dravr-commere`.
//! All notification logic (models, dispatch, triggers, scheduling, Expo Push)
//! lives in the standalone `dravr-commere` crate.

use pierre_core::errors::{AppError, AppResult};

// Re-export all public modules from dravr-commere
pub use dravr_commere::constants;
pub use dravr_commere::expo_push;
pub use dravr_commere::models;
pub use dravr_commere::service;
pub use dravr_commere::triggers;

// Re-export primary public types at crate root
pub use dravr_commere::{
    compute_next_fire_time, validate_cron_expression, CommereError, CommereResult, DispatchOutcome,
    DispatchRequest, NotificationService, SuppressionReason, TenantId,
};

/// Convert a `CommereError` to an `AppError` with structured mapping
#[must_use]
pub fn to_app_error(err: CommereError) -> AppError {
    match err {
        CommereError::Database(msg) => AppError::internal(msg),
        CommereError::PushDelivery { service, message } => {
            AppError::external_service(service, message)
        }
        CommereError::Validation { field, reason } => {
            AppError::invalid_input(format!("{field}: {reason}"))
        }
        CommereError::Scheduling(msg) => AppError::invalid_input(msg),
        CommereError::NotFound { resource } => AppError::not_found(resource),
    }
}

/// Convert a `CommereResult<T>` to an `AppResult<T>`
///
/// # Errors
/// Returns `AppError` mapped from the underlying `CommereError`.
pub fn to_app_result<T>(result: CommereResult<T>) -> AppResult<T> {
    result.map_err(to_app_error)
}
