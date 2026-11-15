// ABOUTME: Pre-approved emails and mailed invitations — the two ways an operator lets an address in before it has an account
// ABOUTME: InvitationKind says what redeeming an invitation gives besides the account (a coach seat, carnet#1524)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::AppError;

/// A standing per-email pre-approval recorded by an operator before the person
/// has an account.
///
/// The registration approval decision consults this allow-list so an allowed
/// address lands [`UserStatus::Active`](crate::models::UserStatus::Active) without the pending queue, and
/// `allowed_by` feeds the new account's `approved_by` for audit attribution.
/// Managed by `pierre-cli user allow / disallow / list-allowed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreApprovedEmail {
    /// The allowed address, stored lowercase.
    pub email: String,
    /// The operator who recorded the allow, when known (`None` pre-bootstrap).
    pub allowed_by: Option<Uuid>,
    /// Operator note (cohort, reason).
    pub note: Option<String>,
    /// When the allow was recorded.
    pub created_at: DateTime<Utc>,
}

/// A single-use invitation an operator mailed to an address with no account.
///
/// The token behind it is never part of this record: only its selector and
/// the SHA-256 of its verifier are stored, and both stay inside the
/// repository. Redeeming the invitation creates the account
/// [`UserStatus::Active`](crate::models::UserStatus::Active), with the invitee's own password and `approved_by`
/// set to `invited_by` while that operator's account still exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInvitation {
    /// The invited address, stored lowercase.
    pub email: String,
    /// The operator who sent it, when attributable (`None` for a service token).
    pub invited_by: Option<Uuid>,
    /// When the link stops working.
    pub expires_at: DateTime<Utc>,
    /// When it was sent.
    pub created_at: DateTime<Utc>,
    /// What redeeming it gives besides the account.
    pub kind: InvitationKind,
}

/// What redeeming an invitation gives besides the account itself.
///
/// Stored in `user_invitations.kind`: NULL for [`Self::Account`], so every
/// invitation sent before the column existed reads as one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationKind {
    /// An account, and nothing else.
    #[default]
    Account,
    /// An account with coach access (`manages_roster`, granted as an operator
    /// grant by the inviter) and the coach role recorded (carnet#1524): the
    /// super-admin decision ADR-018 asks for, taken before sign-up rather
    /// than after a request.
    CoachSeat,
}

impl InvitationKind {
    /// The `kind` column's value: NULL for a plain account invitation.
    #[must_use]
    pub const fn stored(self) -> Option<&'static str> {
        match self {
            Self::Account => None,
            Self::CoachSeat => Some("coach_seat"),
        }
    }

    /// The kind a stored `kind` value names.
    ///
    /// # Errors
    ///
    /// Returns a database error for a value no kind stores.
    pub fn from_stored(raw: Option<&str>) -> Result<Self, AppError> {
        match raw {
            None => Ok(Self::Account),
            Some("coach_seat") => Ok(Self::CoachSeat),
            Some(other) => Err(AppError::database(format!(
                "user_invitations kind holds an unknown value: {other}"
            ))),
        }
    }

    /// The name the operators' notices and the link use for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::CoachSeat => "coach_seat",
        }
    }
}
