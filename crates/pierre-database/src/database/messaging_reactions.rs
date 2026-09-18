// ABOUTME: SQLite lookup resolving an inbound emoji reaction to the chat message it rates
// ABOUTME: Shell over the shared body; SQLite needs no text coercion on user_id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reaction → chat-message resolution for the `SQLite` backend.
//!
//! The statement, the filters and the row decode live in
//! [`crate::repositories::messaging_reactions`]. `messaging_sessions.user_id`
//! is a `TEXT` column here, so the shared body is expanded with an empty cast
//! suffix.

use pierre_core::errors::{AppError, AppResult};
use sqlx::{Pool, Row, Sqlite};

use crate::repositories::messaging_reactions::{
    impl_find_reaction_feedback_target, reaction_target_sql,
};
use crate::repositories::ReactionFeedbackTarget;

/// This backend's resolved lookup: the shared statement with the text
/// coercion its `messaging_sessions.user_id` column needs.
const REACTION_TARGET_SQL: &str = reaction_target_sql!("");

impl_find_reaction_feedback_target!(Sqlite, REACTION_TARGET_SQL);
