// ABOUTME: PostgreSQL lookup resolving an inbound emoji reaction to the chat message it rates
// ABOUTME: Shell over the shared body; Postgres coerces the uuid user_id column to text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reaction → chat-message resolution for the `PostgreSQL` backend.
//!
//! The statement, the filters and the row decode live in
//! [`crate::repositories::messaging_reactions`]. `messaging_sessions.user_id`
//! is a `uuid` column here and the target field is a `String`, so the shared
//! body is expanded with a `::text` cast suffix, which also types the nullable
//! chat-id parameter.

use pierre_core::errors::{AppError, AppResult};
use sqlx::{Pool, Postgres, Row};

use crate::repositories::messaging_reactions::{
    impl_find_reaction_feedback_target, reaction_target_sql,
};
use crate::repositories::ReactionFeedbackTarget;

/// This backend's resolved lookup: the shared statement with the text
/// coercion its `messaging_sessions.user_id` column needs.
const REACTION_TARGET_SQL: &str = reaction_target_sql!("::text");

impl_find_reaction_feedback_target!(Postgres, REACTION_TARGET_SQL);
