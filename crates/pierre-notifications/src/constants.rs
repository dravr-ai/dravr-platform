// ABOUTME: Notification-specific constants for fan-out page size, frequency caps, and schedule limits
// ABOUTME: Centralized configuration defaults that can be overridden via environment variables
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Notification Constants
//!
//! Default configuration values for the notification system.
//! Each constant can be overridden via the corresponding environment variable.

/// Page size for friend fan-out queries (e.g., notifying friends of a shared insight)
/// Set `NOTIFICATION_FAN_OUT_PAGE_SIZE` to override.
pub const FAN_OUT_PAGE_SIZE: i64 = 100;

/// Default daily cap per notification category when user has no preference set.
/// Set `NOTIFICATION_DEFAULT_MAX_PER_DAY` to override.
pub const DEFAULT_MAX_PER_DAY: i64 = 50;

/// Maximum scheduled notifications a user can create within a tenant.
/// Set `NOTIFICATION_MAX_SCHEDULES_PER_USER` to override.
pub const MAX_SCHEDULES_PER_USER: i64 = 20;
