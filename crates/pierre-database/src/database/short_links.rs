// ABOUTME: SQLite-backed ShortLinkRepository, emitted from the shared implementation in repositories/short_links.rs
// ABOUTME: code → target_url with an integer-epoch TTL; resolution filters expired rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::short_links::{
    impl_short_link_repository, target_url_from_row, ShortLinkRepository, INSERT_SHORT_LINK_SQL,
    RESOLVE_SHORT_LINK_SQL, SWEEP_SHORT_LINKS_SQL,
};

impl_short_link_repository!(Database);
