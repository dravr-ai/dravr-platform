// ABOUTME: The stored half of a first-party refresh token — who it belongs to, which rotation chain, and when it lapses
// ABOUTME: The token itself is never part of this record; repositories take it separately and store only its HMAC
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// What the server keeps about a refresh token issued to a first-party device.
///
/// One record per token. A login starts a new `family_id`; every refresh stores
/// a new record in the same family and revokes the one it replaced, so the
/// family is the device session and the record is its current credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRefreshToken {
    /// The rotation chain this token belongs to — one per login.
    pub family_id: String,
    /// The user the token authenticates.
    pub user_id: Uuid,
    /// The tenant the session's JWTs are minted for, as the login resolved it.
    pub tenant_id: Option<String>,
    /// When this token was issued.
    pub created_at: DateTime<Utc>,
    /// When this token stops being exchangeable, whether or not it was used.
    pub expires_at: DateTime<Utc>,
}
