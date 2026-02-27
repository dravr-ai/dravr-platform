// ABOUTME: Messaging repository dispatch for the database factory
// ABOUTME: Delegates MessagingRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::plugins::MessagingRepository;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::messaging::{
    ChannelBindingRecord, CreateChannelBindingParams, CreateMessagingConnectionParams,
    MessagingConnectionRecord,
};
use pierre_core::models::TenantId;

#[async_trait]
impl MessagingRepository for Database {
    async fn create_messaging_connection(
        &self,
        params: &CreateMessagingConnectionParams<'_>,
    ) -> AppResult<MessagingConnectionRecord> {
        match self {
            Self::SQLite(db) => db.create_messaging_connection(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn get_messaging_connection(
        &self,
        id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<MessagingConnectionRecord>> {
        match self {
            Self::SQLite(db) => db.get_messaging_connection(id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn get_messaging_connection_by_team(
        &self,
        provider: &str,
        team_id: &str,
    ) -> AppResult<Option<MessagingConnectionRecord>> {
        match self {
            Self::SQLite(db) => db.get_messaging_connection_by_team(provider, team_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn list_messaging_connections(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessagingConnectionRecord>> {
        match self {
            Self::SQLite(db) => db.list_messaging_connections(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn delete_messaging_connection(
        &self,
        id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete_messaging_connection(id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn create_channel_binding(
        &self,
        params: &CreateChannelBindingParams<'_>,
    ) -> AppResult<ChannelBindingRecord> {
        match self {
            Self::SQLite(db) => db.create_channel_binding(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn get_channel_binding_by_channel(
        &self,
        connection_id: &str,
        channel_id: &str,
    ) -> AppResult<Option<ChannelBindingRecord>> {
        match self {
            Self::SQLite(db) => {
                db.get_channel_binding_by_channel(connection_id, channel_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn list_channel_bindings(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ChannelBindingRecord>> {
        match self {
            Self::SQLite(db) => db.list_channel_bindings(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }

    async fn delete_channel_binding(
        &self,
        id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete_channel_binding(id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => {
                Err(pierre_core::errors::AppError::internal(
                    "Messaging repository not yet implemented for PostgreSQL",
                ))
            }
        }
    }
}
