// ABOUTME: Push notification service crate with SQLite and PostgreSQL backends
// ABOUTME: Provides NotificationService facade encapsulating persistence, dispatch, and scheduling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Notifications
//!
//! Self-contained notification service for the Pierre platform.
//! Owns its own persistence behind a `NotificationService` facade with
//! `SQLite` and `PostgreSQL` backends.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use pierre_notifications::NotificationService;
//!
//! // SQLite backend
//! let service = NotificationService::from_sqlite(pool);
//!
//! // PostgreSQL backend
//! let service = NotificationService::from_postgres(pool);
//!
//! // Start the background scheduler
//! let abort_handle = service.start_scheduler();
//! ```

pub mod constants;
pub mod expo_push;
pub mod service;
pub mod triggers;

pub(crate) mod dispatch;
pub(crate) mod repository;
pub(crate) mod scheduler;

// Re-export primary public types at crate root
pub use dispatch::{DispatchOutcome, DispatchRequest, SuppressionReason};
pub use scheduler::{compute_next_fire_time, validate_cron_expression};
pub use service::NotificationService;
