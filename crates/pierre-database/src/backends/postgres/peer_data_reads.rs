// ABOUTME: PostgreSQL-backed PeerDataReadRepository, emitted from the shared body in repositories/peer_data_reads.rs
// ABOUTME: Postgres stores user ids as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::coach_data_reads::parse_transport;
use crate::repositories::peer_data_reads::{
    impl_peer_data_read_repository, peer_read_error, NewPeerDataRead, PeerDataRead,
    PeerDataReadRepository, LIST_PEER_READS_SINCE_SQL, PURGE_PEER_READS_SQL, RECORD_PEER_READ_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_peer_data_read_repository!(PostgresDatabase, PgRow, NativeUuid);
