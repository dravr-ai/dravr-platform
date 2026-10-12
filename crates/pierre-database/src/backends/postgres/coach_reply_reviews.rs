// ABOUTME: PostgreSQL-backed CoachReplyReviewRepository, emitted from the shared implementation in repositories/coach_reply_reviews.rs
// ABOUTME: Supplies the Postgres row type and the native uuid codec; everything else is shared

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CoachReply, CoachReplyCorrection, CoachReplyFilter, CoachReplyFlag, CoachReplyReview,
    NewReplyCorrection, NewReplyFlag, ReplyFlagReason, ReplyReviewKind, ReplyReviewStamp, TenantId,
};
use pierre_core::transport::TransportPolicy;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::coach_reply_reviews::{
    impl_coach_reply_review_repository, GET_COACH_REPLY_SQL, INSERT_CORRECTION_ENTRY_SQL,
    INSERT_CORRECTION_REVIEW_SQL, INSERT_FLAG_SQL, LIST_COACH_REPLIES_SQL,
    LIST_REVIEWS_FOR_WINDOW_SQL, LIST_REVIEW_STAMPS_SQL,
};
use crate::repositories::coaching_groups::{column, instant};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::CoachReplyReviewRepository;

impl_coach_reply_review_repository!(PostgresDatabase, PgRow, NativeUuid);
