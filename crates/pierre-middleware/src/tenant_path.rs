// ABOUTME: Axum extractor for a tenant id supplied in the URL path
// ABOUTME: Names the client-input origin at the handler signature

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;

/// A [`TenantId`] taken from the URL path.
///
/// Handlers used to write `Path<TenantId>`, which had the framework build a
/// tenant identity out of a URL segment with no conversion visible anywhere in
/// the source. `TenantId` no longer implements `Deserialize`, so that spelling
/// does not compile; this extractor replaces it and says at the signature where
/// the value came from.
///
/// The name is the point: **the path is client input**. Extracting one proves
/// only that the segment was a well-formed UUID. Authorization is a separate
/// check the handler still owes.
pub struct TenantPath(pub TenantId);

impl<S> FromRequestParts<S> for TenantPath
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::invalid_input("Missing tenant ID in path"))?;
        TenantId::parse_str(&raw)
            .map(Self)
            .map_err(|_| AppError::invalid_input("Invalid tenant ID in path"))
    }
}
