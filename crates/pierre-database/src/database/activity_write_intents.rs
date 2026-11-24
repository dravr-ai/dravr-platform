// ABOUTME: SQLite-backed ActivityWriteIntentRepository, emitted from the shared body in repositories/
// ABOUTME: Walk steps' writes to the activity store, recorded before they land, until the step or the sweep settles them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

use crate::database::Database;
use crate::repositories::activity_write_intents::{
    encode_id_list, impl_activity_write_intent_repository, write_intent_from_row,
    ActivityWriteIntent, ActivityWriteIntentRepository, CLEAR_WRITE_INTENT_SQL,
    DUE_WRITE_INTENTS_SQL, POSTPONE_WRITE_INTENT_SQL, RECORD_WRITE_INTENT_SQL,
};

impl_activity_write_intent_repository!(Database);
