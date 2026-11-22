// ABOUTME: Signup codes — the shareable code an admin mints, and the one redemption an account holds
// ABOUTME: A coach's code is a group invite with approves_signup set; a redemption names either source (carnet#1128)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A code an admin minted so that whoever enters it is approved.
///
/// The code itself is never part of this record: only its SHA-256 is stored,
/// with [`Self::code_hint`] (its last four characters) for telling codes
/// apart in a listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupCode {
    /// The code's id
    pub id: Uuid,
    /// The code's last four characters
    pub code_hint: String,
    /// The cohort or reason the admin recorded
    pub note: Option<String>,
    /// The admin who minted it, when attributable (`None` for a service token)
    pub created_by: Option<Uuid>,
    /// When it stops working; `None` never expires
    pub expires_at: Option<Timestamp>,
    /// How many accounts it may approve; `None` is unlimited
    pub max_uses: Option<i32>,
    /// How many accounts it has approved
    pub use_count: i32,
    /// When the admin withdrew it
    pub revoked_at: Option<Timestamp>,
    /// When it was minted
    pub created_at: Timestamp,
}

impl SignupCode {
    /// Whether one more account can redeem the code at `now`.
    #[must_use]
    pub fn is_live(&self, now: Timestamp) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|at| at > now)
            && self.max_uses.is_none_or(|max| self.use_count < max)
    }
}

/// What an account redeemed: an admin's code, or a coach's approving group
/// invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SignupCodeSource {
    /// A row of `signup_codes`
    AdminCode(Uuid),
    /// A row of `group_invites` with `approves_signup` set
    CoachInvite(Uuid),
}

/// The one code an account redeemed, and whether it has activated the
/// account yet.
///
/// A redemption is recorded when the code is entered; the account is
/// activated once its address is proven, which may be at once (a Google
/// sign-in, an address already confirmed) or when the confirmation link lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupCodeRedemption {
    /// The account that entered the code
    pub user_id: Uuid,
    /// The code it entered; `None` once that code's row was deleted
    pub source: Option<SignupCodeSource>,
    /// The admin or coach who minted the code
    pub granted_by: Option<Uuid>,
    /// When the code was entered
    pub redeemed_at: Timestamp,
    /// When the redemption activated the account
    pub activated_at: Option<Timestamp>,
}

/// A group invite an account opened before it could join, held until the
/// account is active (carnet#1648).
///
/// A `/groups/join/<code>` link opened signed out reaches the server with the
/// sign-up, or right after the sign-in, and waits here: the account joins the
/// invite's group once it is active. One hold per account; a newer link
/// replaces the older one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupInviteHold {
    /// The account the invite waits for
    pub user_id: Uuid,
    /// The group invite it opened, a member's or the group's coach seat
    pub invite_id: Uuid,
    /// When the server took it
    pub held_at: Timestamp,
    /// When a join was refused because the group was full: the hold is kept
    /// for a retry once a seat opens (carnet#1650); `None` until then
    pub refused_full_at: Option<Timestamp>,
}

/// How far one group invite got, read from the invite's open counter and the
/// holds that name it (carnet#1650): the numbers behind the coach's pending
/// row. No reader is kept, only counts and times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupInviteFunnelRow {
    /// The invite
    pub invite_id: Uuid,
    /// Times its public card was opened, by anyone but the people running
    /// the group
    pub open_count: i64,
    /// When the card was last opened
    pub last_opened_at: Option<Timestamp>,
    /// Live holds naming the invite: accounts that came from the link and
    /// have not joined
    pub held_count: i64,
    /// When the latest of those holds was taken
    pub last_held_at: Option<Timestamp>,
    /// Of those holds, how many a full group refused
    pub full_count: i64,
}
