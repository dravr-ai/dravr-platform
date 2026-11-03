// ABOUTME: Where each key version's wrapped bytes are stored, and how one is read and unwrapped with the KEK
// ABOUTME: Shared by the loader, the controller's staging, the CLI's adopt and rewrap, and the legacy-layout reader

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as Base64Engine;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::redaction::redact_url;
use pierre_database::backends::shared::key_material::{
    key_read_event, DEK_READ_FAILED, DEK_UNWRAP_FAILED,
};
use pierre_database::backends::shared::keyring::{KeyFamily, KEY_LEN};
use pierre_database::backends::SecurityRepository;
use tracing::error;

use crate::key_management::KekProvider;

/// The version-1 data key's row, which is also index v1.
pub const V1_SECRET: &str = "database_encryption_key";

/// The legacy layout's active data version. Deleted at adoption; nothing
/// reads it once a keyset document exists.
pub const ACTIVE_VERSION_SECRET: &str = "database_encryption_key_active_version";

/// The row a key version's wrapped bytes are stored in. Data v1 keeps its
/// historical name, and index v1 is that same row.
#[must_use]
pub fn secret_name(family: KeyFamily, version: u32) -> String {
    match (family, version) {
        (_, 1) => V1_SECRET.to_owned(),
        (KeyFamily::Data, n) => format!("database_encryption_key_v{n}"),
        (KeyFamily::Index, n) => format!("blind_index_key_v{n}"),
    }
}

/// Read the legacy layout's active data version, 1 when the secret is
/// absent. Only a definite not-found means version 1; a failed read is
/// returned, since guessing 1 would hide every newer key.
///
/// # Errors
///
/// When the read fails or the value is not a version number.
pub async fn read_legacy_active_version(security: &dyn SecurityRepository) -> AppResult<u32> {
    let value = match security.get_system_secret(ACTIVE_VERSION_SECRET).await {
        Ok(value) => value,
        Err(e) if e.code == ErrorCode::ResourceNotFound => return Ok(1),
        Err(e) => {
            if let Some(event) = key_read_event(&e, DEK_READ_FAILED) {
                error!(event, error_code = ?e.code, error = %e, "Failed to read the active DEK version");
            }
            return Err(e);
        }
    };
    let version = value.trim().parse::<u32>().map_err(|e| {
        let invalid =
            AppError::internal(format!("Invalid stored active DEK version '{value}': {e}"));
        error!(
            event = DEK_READ_FAILED,
            error_code = ?invalid.code,
            error = %invalid,
            "Stored active DEK version is not a version number"
        );
        invalid
    })?;
    if version == 0 {
        let invalid = AppError::internal("Stored active DEK version is 0; versions start at 1");
        error!(
            event = DEK_READ_FAILED,
            dek_version = 0,
            error = %invalid,
            "Stored active DEK version names no stored key"
        );
        return Err(invalid);
    }
    Ok(version)
}

/// Read and unwrap one key version.
///
/// # Errors
///
/// When the row cannot be read, is not base64, does not unwrap with the KEK,
/// or unwraps to a key of the wrong length; each is logged under its
/// `key_material.*` event.
pub async fn load_key(
    security: &dyn SecurityRepository,
    kek: &dyn KekProvider,
    family: KeyFamily,
    version: u32,
) -> AppResult<Vec<u8>> {
    let name = secret_name(family, version);
    let wrapped_base64 = security.get_system_secret(&name).await.inspect_err(|e| {
        if let Some(event) = key_read_event(e, DEK_READ_FAILED) {
            error!(event, key_family = %family, key_version = version, error_code = ?e.code, error = %e, "Failed to read a stored key version");
        }
    })?;
    let wrapped = Base64Standard.decode(&wrapped_base64).map_err(|e| {
        let invalid = AppError::internal(format!("Invalid base64 encoding for stored key {name}: {e}"));
        error!(event = DEK_READ_FAILED, key_family = %family, key_version = version, error = %invalid, "Stored key version is not valid base64");
        invalid
    })?;
    let unwrapped = kek.unwrap(&wrapped).await.map_err(|e| {
        error!(event = DEK_UNWRAP_FAILED, key_family = %family, key_version = version, error_code = ?e.code, error = %e, "Stored key failed to unwrap with the key-encryption key");
        if e.message.contains("Decryption failed") {
            let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "unknown".to_owned());
            AppError::encryption_key_mismatch(&redact_url(&database_url))
        } else {
            e
        }
    })?;
    if unwrapped.len() != KEY_LEN {
        let invalid = AppError::internal(format!(
            "Unwrapped {family} key v{version} has invalid length: expected {KEY_LEN} bytes, got {}",
            unwrapped.len()
        ));
        error!(event = DEK_UNWRAP_FAILED, key_family = %family, key_version = version, error = %invalid, "Stored key unwrapped to a key of the wrong length");
        return Err(invalid);
    }
    Ok(unwrapped)
}

/// Store a new key version's wrapped bytes, insert-if-absent; whether this
/// call's bytes are the ones stored (another writer's win means `false`).
///
/// # Errors
///
/// When the write fails.
pub async fn insert_key_if_absent(
    security: &dyn SecurityRepository,
    family: KeyFamily,
    version: u32,
    wrapped: &[u8],
) -> AppResult<bool> {
    let encoded = Base64Standard.encode(wrapped);
    let stored = security
        .insert_system_secret_if_absent(&secret_name(family, version), &encoded)
        .await?;
    Ok(stored == encoded)
}

/// Replace a key version's wrapped bytes (KEK rotation only: the key inside
/// is unchanged).
///
/// # Errors
///
/// When the write fails.
pub async fn rewrite_key(
    security: &dyn SecurityRepository,
    family: KeyFamily,
    version: u32,
    wrapped: &[u8],
) -> AppResult<()> {
    security
        .update_system_secret(
            &secret_name(family, version),
            &Base64Standard.encode(wrapped),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_is_one_row_for_both_families_and_later_versions_are_apart() {
        assert_eq!(secret_name(KeyFamily::Data, 1), V1_SECRET);
        assert_eq!(secret_name(KeyFamily::Index, 1), V1_SECRET);
        assert_eq!(
            secret_name(KeyFamily::Data, 2),
            "database_encryption_key_v2"
        );
        assert_eq!(secret_name(KeyFamily::Index, 2), "blind_index_key_v2");
    }
}
