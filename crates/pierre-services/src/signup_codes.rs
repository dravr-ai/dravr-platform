// ABOUTME: Signup codes — an admin mints, lists and revokes them; one place resolves and claims a code someone entered
// ABOUTME: Both admin surfaces (pierre-cli, admin web) and both redemption doors reach codes through here (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Signup codes.
//!
//! A signup code approves whoever enters it. Two kinds share one entry field:
//!
//! - an **admin's code**, minted here and stored in `signup_codes` only as
//!   its SHA-256: the code exists once, in the [`MintedSignupCode`] handed to
//!   the admin, and a listing shows its last four characters;
//! - a **coach's code**, a member invite of the coach's group with
//!   `approves_signup` set ([`pierre_groups::GroupService::create_signup_invite`]),
//!   which also joins the coach's group.
//!
//! Whatever was entered is normalized first ([`normalize_code`]): case and
//! separators do not matter, so a code read aloud or retyped from a slide
//! still matches. Entering one is claimed in a single conditional statement
//! per kind, so the last use of a code cannot be taken twice. Nothing here
//! logs a code.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{GroupInvite, GroupInviteKind};
use pierre_core::models::{SignupCode, SignupCodeSource};
use pierre_database::RepositoryRegistry;
use pierre_groups::unambiguous_code;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

/// Random characters in an admin's code: 12 of a 32-symbol alphabet is 60
/// bits, so guessing one is out of reach behind the per-address limiter.
const CODE_RANDOM_LEN: usize = 12;

/// Longest vanity prefix an admin may put in front of the random part.
const MAX_PREFIX_LEN: usize = 12;

/// The most accounts one admin code may approve.
pub const MAX_USES_LIMIT: i32 = 10_000;

/// The longest an admin code may stay redeemable, in days.
pub const MAX_EXPIRY_DAYS: i64 = 365;

/// The `details.reason` an invalid code's refusal carries, so a client words
/// it in the reader's language instead of matching the English message.
pub const INVALID_SIGNUP_CODE_REASON: &str = "invalid_signup_code";

/// The one answer to a code that is unknown, revoked, expired or used up, so
/// the door never tells which.
#[must_use]
pub fn invalid_signup_code() -> AppError {
    let mut refusal = AppError::invalid_input("This code isn't valid. Check it and try again.");
    refusal.details = Some(Box::new(
        serde_json::json!({ "reason": INVALID_SIGNUP_CODE_REASON }),
    ));
    refusal
}

/// The form a code is compared in: letters and digits only, upper-case.
#[must_use]
pub fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// SHA-256 hex of a normalized code: the only form `signup_codes` stores.
#[must_use]
pub fn hash_code(normalized: &str) -> String {
    hex::encode(Sha256::digest(normalized.as_bytes()))
}

/// A freshly minted admin code: what the admin is shown, once.
///
/// Deliberately not `Debug`: `code` is the only copy of the secret.
pub struct MintedSignupCode {
    /// The code to hand out, grouped by dashes for reading
    pub code: String,
    /// The stored record
    pub record: SignupCode,
}

impl fmt::Debug for MintedSignupCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MintedSignupCode")
            .field("code", &"[REDACTED]")
            .field("record", &self.record)
            .finish()
    }
}

/// What an admin asks for when minting a code.
#[derive(Debug, Clone, Default)]
pub struct MintRequest {
    /// The cohort or reason, shown in the listing
    pub note: Option<String>,
    /// A readable prefix (letters and digits) in front of the random part
    pub prefix: Option<String>,
    /// How many accounts it may approve; `None` is unlimited
    pub max_uses: Option<i32>,
    /// Days until it stops working; `None` never expires
    pub expires_in_days: Option<i64>,
}

/// Mint an admin's signup code.
///
/// # Errors
///
/// Returns an invalid-input error for a prefix that is not 1 to 12 letters
/// and digits, a `max_uses` outside `1..=10000` or an expiry outside
/// `1..=365` days, and the repository's error when the insert fails.
pub async fn mint(
    repos: &RepositoryRegistry,
    created_by: Option<Uuid>,
    request: &MintRequest,
) -> AppResult<MintedSignupCode> {
    let prefix = request
        .prefix
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let upper = p.to_ascii_uppercase();
            if upper.len() > MAX_PREFIX_LEN || !upper.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(AppError::invalid_input(format!(
                    "prefix must be 1 to {MAX_PREFIX_LEN} letters and digits"
                )));
            }
            Ok(upper)
        })
        .transpose()?;
    if let Some(max) = request.max_uses {
        if !(1..=MAX_USES_LIMIT).contains(&max) {
            return Err(AppError::invalid_input(format!(
                "max_uses must be between 1 and {MAX_USES_LIMIT}"
            )));
        }
    }
    if let Some(days) = request.expires_in_days {
        if !(1..=MAX_EXPIRY_DAYS).contains(&days) {
            return Err(AppError::invalid_input(format!(
                "expires_in_days must be between 1 and {MAX_EXPIRY_DAYS}"
            )));
        }
    }

    let random = random_code();
    let grouped = random
        .as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join("-");
    let code = prefix.map_or_else(|| grouped.clone(), |p| format!("{p}-{grouped}"));
    let normalized = normalize_code(&code);
    let hint = normalized[normalized.len() - 4..].to_owned();
    let expires_at = request
        .expires_in_days
        .map(|days| Utc::now() + Duration::days(days));
    let note = request
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());

    let record = repos
        .signup_codes
        .mint(
            &hash_code(&normalized),
            &hint,
            note,
            created_by,
            expires_at,
            request.max_uses,
        )
        .await?;
    info!(code_id = %record.id, created_by = ?created_by, "Signup code minted");
    Ok(MintedSignupCode { code, record })
}

/// `CODE_RANDOM_LEN` characters of the unambiguous invite alphabet.
fn random_code() -> String {
    unambiguous_code(CODE_RANDOM_LEN)
}

/// Whether a coach's approving invite can be redeemed at `now`.
fn coach_invite_live(invite: &GroupInvite, now: DateTime<Utc>) -> bool {
    invite.approves_signup
        && invite.kind == GroupInviteKind::Member
        && invite.is_active
        && invite.expires_at.is_none_or(|at| at > now)
        && invite.max_uses.is_none_or(|max| invite.use_count < max)
}

/// Whether `raw` names a code someone could redeem right now, without
/// claiming a use.
///
/// The register form checks this before creating anything,
/// so a mistyped code is answered while the person can still fix it.
///
/// # Errors
///
/// Returns the repository's error when a lookup fails.
pub async fn is_live(repos: &RepositoryRegistry, raw: &str) -> AppResult<bool> {
    let normalized = normalize_code(raw);
    if normalized.is_empty() {
        return Ok(false);
    }
    let now = Utc::now();
    if repos
        .signup_codes
        .find_live(&hash_code(&normalized), now)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(repos
        .groups
        .get_invite_by_code(&normalized)
        .await?
        .is_some_and(|invite| coach_invite_live(&invite, now)))
}

/// One use of a code, claimed for an account.
#[derive(Debug, Clone)]
pub struct ClaimedCode {
    /// What was redeemed
    pub source: SignupCodeSource,
    /// The admin or coach who minted it
    pub granted_by: Option<Uuid>,
    /// The coach's group invite, for a coach's code
    pub coach_invite: Option<GroupInvite>,
}

/// Claim one use of the code `raw` names: `None` when it is unknown,
/// revoked, expired or used up.
///
/// # Errors
///
/// Returns the repository's error when a claim fails.
pub async fn claim(repos: &RepositoryRegistry, raw: &str) -> AppResult<Option<ClaimedCode>> {
    let normalized = normalize_code(raw);
    if normalized.is_empty() {
        return Ok(None);
    }
    let now = Utc::now();
    if let Some(code) = repos
        .signup_codes
        .claim(&hash_code(&normalized), now)
        .await?
    {
        return Ok(Some(ClaimedCode {
            source: SignupCodeSource::AdminCode(code.id),
            granted_by: code.created_by,
            coach_invite: None,
        }));
    }
    Ok(repos
        .groups
        .claim_signup_invite(&normalized, now)
        .await?
        .map(|invite| ClaimedCode {
            source: SignupCodeSource::CoachInvite(invite.id),
            granted_by: Some(invite.created_by),
            coach_invite: Some(invite),
        }))
}

/// Give back a use [`claim`] took for an account that could not record it.
///
/// An admin code's count is decremented; a coach invite's use stays spent,
/// since the invite is single-group and the race that reaches here is an
/// account redeeming twice at once.
pub async fn release(repos: &RepositoryRegistry, claimed: &ClaimedCode) {
    if let SignupCodeSource::AdminCode(id) = claimed.source {
        if let Err(e) = repos.signup_codes.release(id).await {
            warn!(code_id = %id, error = %e, "Could not give back a claimed signup code use");
        }
    }
}

/// Which kind a listed code is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeKind {
    /// Minted by an admin
    Admin,
    /// A coach's approving group invite
    Coach,
}

/// One account that came in on a code.
#[derive(Debug, Clone, Serialize)]
pub struct CodeRedeemer {
    /// The account's email
    pub email: String,
    /// The account's status now
    pub status: String,
    /// When it entered the code
    pub redeemed_at: DateTime<Utc>,
    /// When the code activated it; `None` while it waits on its address
    pub activated_at: Option<DateTime<Utc>>,
}

/// One code as an admin reads it.
#[derive(Debug, Clone, Serialize)]
pub struct CodeListing {
    /// Admin or coach
    pub kind: CodeKind,
    /// The code's id: a `signup_codes` row or a `group_invites` row
    pub id: Uuid,
    /// The code's last four characters
    pub code_hint: String,
    /// The admin's note; `None` for a coach's code
    pub note: Option<String>,
    /// Who minted it
    pub created_by: Option<Uuid>,
    /// Their email, resolved for display
    pub created_by_email: Option<String>,
    /// The coach's group, for a coach's code
    pub group_id: Option<Uuid>,
    /// When it stops working
    pub expires_at: Option<DateTime<Utc>>,
    /// Its use limit
    pub max_uses: Option<i32>,
    /// Its uses so far
    pub use_count: i32,
    /// Whether it can still be redeemed
    pub live: bool,
    /// When it was minted
    pub created_at: DateTime<Utc>,
    /// The accounts that came in on it
    pub redeemed_by: Vec<CodeRedeemer>,
}

/// Every admin code and every coach's approving invite, newest first, with
/// the accounts that came in on each.
///
/// # Errors
///
/// Returns the repository's error when a listing fails. A failed lookup of
/// one minter or one redeemer degrades that field rather than failing it.
pub async fn list(repos: &RepositoryRegistry) -> AppResult<Vec<CodeListing>> {
    let now = Utc::now();
    let mut redeemers: HashMap<SignupCodeSource, Vec<CodeRedeemer>> = HashMap::new();
    for redemption in repos.signup_codes.list_redemptions().await? {
        let Some(source) = redemption.source else {
            continue;
        };
        let (email, status) = match repos.users.get_global(redemption.user_id).await {
            Ok(Some(user)) => (user.email, user.user_status.to_string()),
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %e, "Account lookup failed for a signup code redemption");
                continue;
            }
        };
        redeemers.entry(source).or_default().push(CodeRedeemer {
            email,
            status,
            redeemed_at: redemption.redeemed_at,
            activated_at: redemption.activated_at,
        });
    }

    let mut emails: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut out = Vec::new();
    for code in repos.signup_codes.list().await? {
        let created_by_email = minter_email(repos, &mut emails, code.created_by).await;
        out.push(CodeListing {
            kind: CodeKind::Admin,
            id: code.id,
            live: code.is_live(now),
            code_hint: code.code_hint,
            note: code.note,
            created_by: code.created_by,
            created_by_email,
            group_id: None,
            expires_at: code.expires_at,
            max_uses: code.max_uses,
            use_count: code.use_count,
            created_at: code.created_at,
            redeemed_by: redeemers
                .remove(&SignupCodeSource::AdminCode(code.id))
                .unwrap_or_default(),
        });
    }
    for invite in repos.groups.list_signup_invites().await? {
        let created_by_email = minter_email(repos, &mut emails, Some(invite.created_by)).await;
        let hint_start = invite.code.len().saturating_sub(4);
        out.push(CodeListing {
            kind: CodeKind::Coach,
            id: invite.id,
            live: coach_invite_live(&invite, now),
            code_hint: invite.code[hint_start..].to_owned(),
            note: None,
            created_by: Some(invite.created_by),
            created_by_email,
            group_id: Some(invite.group_id),
            expires_at: invite.expires_at,
            max_uses: invite.max_uses,
            use_count: invite.use_count,
            created_at: invite.created_at,
            redeemed_by: redeemers
                .remove(&SignupCodeSource::CoachInvite(invite.id))
                .unwrap_or_default(),
        });
    }
    out.sort_by_key(|code| Reverse(code.created_at));
    Ok(out)
}

/// Resolve one minter's email through a per-listing cache.
async fn minter_email(
    repos: &RepositoryRegistry,
    cache: &mut HashMap<Uuid, Option<String>>,
    id: Option<Uuid>,
) -> Option<String> {
    let id = id?;
    if let Some(cached) = cache.get(&id) {
        return cached.clone();
    }
    let resolved = match repos.users.get_global(id).await {
        Ok(user) => user.map(|u| u.email),
        Err(e) => {
            warn!(error = %e, "Minter lookup failed for a signup code");
            None
        }
    };
    cache.insert(id, resolved.clone());
    resolved
}

/// Withdraw a code, admin or coach. Returns whether a live one was withdrawn.
/// Accounts it already approved stay approved.
///
/// # Errors
///
/// Returns a not-found error for an id that names no code of that kind, and
/// the repository's error when the write fails.
pub async fn revoke(repos: &RepositoryRegistry, kind: CodeKind, id: Uuid) -> AppResult<bool> {
    let revoked = match kind {
        CodeKind::Admin => {
            let revoked = repos.signup_codes.revoke(id, Utc::now()).await?;
            // Nothing changed: an already-revoked code, or no code at all.
            if !revoked && !repos.signup_codes.list().await?.iter().any(|c| c.id == id) {
                return Err(AppError::not_found("No signup code with that id"));
            }
            revoked
        }
        CodeKind::Coach => {
            let invite = repos
                .groups
                .get_invite_by_id(&id.to_string())
                .await?
                .filter(|invite| invite.approves_signup)
                .ok_or_else(|| AppError::not_found("No coach signup code with that id"))?;
            repos
                .groups
                .deactivate_invite(&invite.group_id.to_string(), &id.to_string())
                .await?
                && invite.is_active
        }
    };
    if revoked {
        info!(code_id = %id, kind = ?kind, "Signup code revoked");
    }
    Ok(revoked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing_ignores_case_spaces_and_dashes() {
        assert_eq!(normalize_code(" dravr-ab2c-def3 "), "DRAVRAB2CDEF3");
        assert_eq!(normalize_code("--"), "");
    }

    #[test]
    fn a_hash_is_stable_and_never_the_code() {
        let hashed = hash_code("ABCD");
        assert_eq!(hashed, hash_code("ABCD"));
        assert_ne!(hashed, "ABCD");
        assert_eq!(hashed.len(), 64);
    }

    #[test]
    fn a_random_code_has_sixty_bits_from_the_unambiguous_alphabet() {
        let code = random_code();
        assert_eq!(code.len(), CODE_RANDOM_LEN);
        assert!(code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
        assert!(!code.contains(['0', 'O', '1', 'I']));
    }
}
