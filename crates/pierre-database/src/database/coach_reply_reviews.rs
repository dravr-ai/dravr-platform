// ABOUTME: SQLite-backed CoachReplyReviewRepository, emitted from the shared implementation in repositories/coach_reply_reviews.rs
// ABOUTME: Supplies the SQLite row type and the text uuid codec; everything else is shared

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::constants::oauth_providers::column_excluded_from_ai_evaluation;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CoachReply, CoachReplyCorrection, CoachReplyFilter, CoachReplyFlag, CoachReplyReview,
    NewReplyCorrection, NewReplyFlag, ReplyFlagReason, ReplyReviewKind, ReplyReviewStamp, TenantId,
};
use pierre_core::transport::TransportPolicy;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::coach_reply_reviews::{
    get_coach_reply_sql, impl_coach_reply_review_repository, list_coach_replies_sql,
    INSERT_CORRECTION_ENTRY_MEMBERS_SQL, INSERT_CORRECTION_ENTRY_SQL, INSERT_CORRECTION_REVIEW_SQL,
    INSERT_FLAG_SQL, LIST_REVIEWS_FOR_WINDOW_SQL, LIST_REVIEW_STAMPS_SQL,
};
use crate::repositories::coaching_groups::{column, instant};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::CoachReplyReviewRepository;

impl_coach_reply_review_repository!(Database, SqliteRow, TextUuid);
