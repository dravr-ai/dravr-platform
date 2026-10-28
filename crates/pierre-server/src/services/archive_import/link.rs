// ABOUTME: The export link an athlete pasted: encrypted with associated data naming their account, never logged
// ABOUTME: Logs and errors carry its SHA-256 instead; a ciphertext moved to another athlete's job does not decrypt

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Custody of the export link
//!
//! A service's export link is a bearer credential: whoever holds it can read
//! the athlete's whole export until it expires. The link is kept only between
//! the athlete pasting it and the download that spools the export (barrage
//! clears it then, or when the job reaches any final state), and only
//! encrypted, with associated data binding the ciphertext to the athlete's
//! tenant and account, the source, and the link's own digest. A ciphertext
//! copied onto another athlete's job does not decrypt.
//!
//! The import id is not part of the binding: barrage assigns it when the job
//! is created, and the link is sealed before, since the job is created with
//! its link in one statement. The digest, stored beside the ciphertext, is
//! what ties a ciphertext to the link it was made from.

use std::str;

use dravr_barrage::digest::Sha256;
use dravr_barrage::{Scope, SourceKey};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::SecurityRepository;

/// The digest of `link`, which logs, errors and the job row carry in its
/// place.
#[must_use]
pub fn link_digest(link: &str) -> Sha256 {
    Sha256::of(link.as_bytes())
}

/// The associated data a link's ciphertext is bound to.
fn associated_data(scope: &Scope, source: SourceKey, digest: &Sha256) -> String {
    format!(
        "barrage.import_jobs.url|{}|{}|{}|{}",
        scope.tenant_text(),
        scope.user_text(),
        source.as_str(),
        digest.to_hex()
    )
}

/// `link` encrypted for `scope`'s import of `source`, as the bytes the job
/// row stores.
///
/// # Errors
///
/// An internal error when encryption fails.
pub fn seal(
    security: &dyn SecurityRepository,
    scope: &Scope,
    source: SourceKey,
    link: &str,
) -> AppResult<Vec<u8>> {
    let aad = associated_data(scope, source, &link_digest(link));
    security
        .encrypt_data_with_aad(link, &aad)
        .map(String::into_bytes)
}

/// The link `ciphertext` holds, when it was sealed for `scope`'s import of
/// `source` from the link `digest` names.
///
/// # Errors
///
/// An internal error when the bytes are not a ciphertext, or were sealed for
/// another athlete, source or link: the associated data no longer matches.
pub fn open(
    security: &dyn SecurityRepository,
    scope: &Scope,
    source: SourceKey,
    digest: &Sha256,
    ciphertext: &[u8],
) -> AppResult<String> {
    let text = str::from_utf8(ciphertext)
        .map_err(|_| AppError::internal("the stored export link is not a ciphertext"))?;
    security.decrypt_data_with_aad(text, &associated_data(scope, source, digest))
}
