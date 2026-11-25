// ABOUTME: The names the session-read satellite owns for its read errors and its busy wire value, in the platform's words
// ABOUTME: The one place the platform names them: every other module imports these aliases
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The platform's names for what the session-read satellite names its own way.
//!
//! A failed session read is the dependency's error type, and a read the
//! service sheds under load carries the dependency's busy wire value. The
//! platform imports them from here, under the names it uses everywhere else.

pub use dravr_sciotte::error::{
    ScraperError as SessionReadError, ScraperResult as SessionReadResult,
};
pub use dravr_sciotte::wire::SCRAPER_BUSY as SERVICE_BUSY;
