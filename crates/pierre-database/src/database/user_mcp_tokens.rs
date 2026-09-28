// ABOUTME: SQLite-backed UserMcpTokenRepository, emitted from the shared body in repositories/user_mcp_tokens.rs
// ABOUTME: SQLite stores user_id as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::user_mcp_tokens::{
    generate_mcp_token, hash_mcp_token, impl_user_mcp_token_repository, mcp_token_column_error,
    mcp_token_prefix, usage_count_from_column, UserMcpTokenRepository, CREATE_MCP_TOKEN_SQL,
    FIND_MCP_TOKEN_BY_VALUE_SQL, GET_MCP_TOKEN_SQL, LIST_MCP_TOKENS_SQL, REVOKE_MCP_TOKEN_SQL,
    TOUCH_MCP_TOKEN_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_user_mcp_token_repository!(Database, SqliteRow, TextUuid);
