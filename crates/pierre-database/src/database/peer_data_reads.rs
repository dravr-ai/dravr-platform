// ABOUTME: SQLite-backed PeerDataReadRepository, emitted from the shared body in repositories/peer_data_reads.rs
// ABOUTME: SQLite stores user ids as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::coach_data_reads::parse_transport;
use crate::repositories::peer_data_reads::{
    impl_peer_data_read_repository, peer_read_error, NewPeerDataRead, PeerDataRead,
    PeerDataReadRepository, LIST_PEER_READS_SINCE_SQL, PURGE_PEER_READS_SQL, RECORD_PEER_READ_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_peer_data_read_repository!(Database, SqliteRow, TextUuid);
