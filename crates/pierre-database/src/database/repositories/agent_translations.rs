// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The SQLite read behind every localized agent surface — one query per page of agent ids
// ABOUTME: Mirrors backends/postgres/agent_translations.rs; the overlay itself lives in pierre-core

//! Per-locale agent overlay rows, read from `agent_translations`.
//!
//! Split out of `agents_impl` so the file stays under the size ceiling and
//! the SQLite reader sits beside its Postgres twin rather than inside the
//! 1200-line repository implementation.

use std::collections::HashMap;
use std::fmt::Write;

use sqlx::Row;

use crate::database::Database;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::AgentFieldOverlay;

impl Database {
    /// The `agent_translations` overlays for `ids` in `locale`, keyed by agent id.
    ///
    /// English is canonical — it lives on the `agents` row itself — so the
    /// read is skipped for `en` and for an empty id list. Both translation
    /// readers above go through here, so the store and the chat list overlay
    /// the same rows the same way.
    pub(super) async fn agent_translation_overlays(
        &self,
        ids: &[String],
        locale: &str,
    ) -> AppResult<HashMap<String, AgentFieldOverlay>> {
        let mut overlays = HashMap::new();
        if locale == "en" || ids.is_empty() {
            return Ok(overlays);
        }

        // SQLite lacks `= ANY($2)`; build an IN list with one bind per agent.
        // Agent counts per read are bounded by the caller's page size (at most
        // 100) so the placeholder growth stays well under SQLite's 32766 cap.
        let mut query_str = String::from(
            "SELECT agent_id, title, description, purpose, instructions, tags \
             FROM agent_translations WHERE locale = ?1 AND agent_id IN (",
        );
        for i in 0..ids.len() {
            if i > 0 {
                query_str.push(',');
            }
            // SQLite positional binds are 1-indexed; reserve slot 1 for locale.
            let _ = write!(query_str, "?{}", i + 2);
        }
        query_str.push(')');

        let mut q = sqlx::query(&query_str).bind(locale);
        for id in ids {
            q = q.bind(id);
        }
        let rows = q
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to load coach translations: {e}")))?;

        for row in &rows {
            let id: String = row.get("agent_id");
            overlays.insert(
                id,
                AgentFieldOverlay {
                    title: row.try_get("title").ok(),
                    description: row.try_get("description").ok(),
                    purpose: row.try_get("purpose").ok(),
                    instructions: row.try_get("instructions").ok(),
                    tags: row
                        .try_get::<Option<String>, _>("tags")
                        .ok()
                        .flatten()
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                },
            );
        }
        Ok(overlays)
    }
}
