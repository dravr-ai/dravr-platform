// ABOUTME: Standing per-email pre-approval list — allow / disallow / list as one shared operation set
// ABOUTME: Both admin HTTP surfaces and pierre-cli reach the list through here, never the repository directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin management of the `pre_approved_emails` allow-list.
//!
//! An allow is standing: registration consults the list
//! ([`crate::auth::AuthService`]) so an allowed address lands
//! [`UserStatus::Active`] in one step with `approved_by` attributed to the
//! admin who recorded it, instead of queueing for approval.
//!
//! The three verbs live here rather than in a route handler because two
//! surfaces drive them — the bearer-token admin routes behind
//! `pierre-cli user allow / disallow / list-allowed`, and the cookie-authenticated
//! admin web app — and an allow must mean the same thing on both. In
//! particular, allowing an address that already has a *pending* account
//! approves it now (default MCP token included), which is the case an
//! admin hits whenever the person registered before being told not to.

use std::collections::HashMap;

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{normalize_email, UserStatus};
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::admin_ops::{
    activate_pending_sign_up, create_default_mcp_token_for_user, AccountCaller, ActingOn,
};
use crate::auth::AuthService;
use crate::invitation;

/// The address in its stored form ([`normalize_email`]), rejecting one that is
/// not an email.
///
/// The table is keyed on that form, so normalizing at the edge is what makes
/// `Allow` then `Disallow` of the same address with different capitalization
/// refer to one row, and what the result reports back to the admin.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when the value is not a valid email
/// address — rejected before it reaches the table, where a typo would sit as a
/// permanent allow nobody can match against.
pub(crate) fn valid_email(email: &str) -> AppResult<String> {
    let normalized = normalize_email(email);
    if !AuthService::is_valid_email(&normalized) {
        return Err(AppError::invalid_input(format!(
            "'{normalized}' is not a valid email address"
        )));
    }
    Ok(normalized)
}

/// What an allow did — the account state it found, and what it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowOutcome {
    /// No account yet: a standing pre-approval was recorded.
    Recorded,
    /// No account yet, and the address was already on the list.
    AlreadyAllowed,
    /// A pending account existed and was approved now.
    PendingApproved,
    /// An active account already exists; nothing to do.
    AlreadyActive,
    /// A suspended account exists and was deliberately left alone.
    SuspendedUnchanged,
}

/// The account an allow promoted out of the pending queue.
///
/// Carries what an approval announcement needs, so a caller holding a
/// [`crate::user_approval::UserApprovalNotifier`] never has to re-read the
/// user it just approved.
#[derive(Debug, Clone)]
pub struct ApprovedUser {
    /// The promoted user's id.
    pub id: Uuid,
    /// Their account email.
    pub email: String,
    /// Their display name, when set.
    pub display_name: Option<String>,
}

/// Outcome of [`allow`], carrying what the admin needs to be told and what
/// the caller needs in order to notify a just-approved user.
#[derive(Debug, Clone)]
pub struct AllowResult {
    /// The normalized address the allow applies to.
    pub email: String,
    /// What happened.
    pub outcome: AllowOutcome,
    /// The account promoted out of the pending queue, when
    /// [`AllowOutcome::PendingApproved`].
    pub approved_user: Option<ApprovedUser>,
}

impl AllowResult {
    /// Admin-facing sentence describing the outcome.
    ///
    /// One wording, produced server-side, so the CLI and the admin web app
    /// cannot describe the same result differently.
    #[must_use]
    pub fn message(&self) -> String {
        let email = &self.email;
        match self.outcome {
            AllowOutcome::Recorded => format!(
                "{email} pre-approved — their registration will land active (no approval queue)"
            ),
            AllowOutcome::AlreadyAllowed => format!("{email} is already pre-approved (no change)"),
            AllowOutcome::PendingApproved => {
                format!("{email} had a pending account — approved now (status: active)")
            }
            AllowOutcome::AlreadyActive => {
                format!("{email} already has an active account (no change)")
            }
            AllowOutcome::SuspendedUnchanged => format!(
                "{email} has a suspended account — left unchanged. \
                 Reinstate explicitly with: pierre-cli user approve --email {email}"
            ),
        }
    }
}

/// Outcome of [`disallow`].
#[derive(Debug, Clone)]
pub struct DisallowResult {
    /// The normalized address.
    pub email: String,
    /// Whether a row was actually removed.
    pub removed: bool,
    /// Whether a live invitation to the address was revoked with it.
    pub invitation_revoked: bool,
    /// Status of an account that exists for this address, if any. Removing a
    /// pre-approval never changes it, and an admin who expects otherwise
    /// needs to see that it did not.
    pub account_status: Option<String>,
}

impl DisallowResult {
    /// Admin-facing sentence describing the outcome.
    #[must_use]
    pub fn message(&self) -> String {
        let email = &self.email;
        let outcome = if self.removed {
            format!("{email} removed from the pre-approved list")
        } else {
            format!("{email} was not on the pre-approved list (nothing to remove)")
        };
        let invitation = if self.invitation_revoked {
            " — its invitation link no longer works"
        } else {
            ""
        };
        let note = self
            .account_status
            .as_deref()
            .map_or_else(String::new, |status| {
                format!(
                    " — note: an account already exists for {email} (status: {status}), \
                 which disallow does not change"
                )
            });
        format!("{outcome}{invitation}{note}")
    }
}

/// One pre-approved address with the state an admin reads it for.
#[derive(Debug, Clone, Serialize)]
pub struct AllowedEmail {
    /// The allowed address, lower-case.
    pub email: String,
    /// Admin note recorded with the allow (cohort, reason).
    pub note: Option<String>,
    /// When the allow was recorded.
    pub created_at: Timestamp,
    /// The admin who recorded it, when attributable.
    pub allowed_by: Option<Uuid>,
    /// That admin's email, resolved for display.
    pub allowed_by_email: Option<String>,
    /// Status of the account registered against this address, or `None` when
    /// nobody has registered yet — the whole point of a standing allow is that
    /// it outlives the wait, so "not yet" is a normal steady state.
    pub account_status: Option<String>,
    /// When the address's outstanding invitation link stops working, or `None`
    /// when no invitation is waiting to be redeemed.
    pub invitation_expires_at: Option<Timestamp>,
}

/// Record a standing pre-approval, approving a pending account on the spot.
///
/// Four cases, and they are the reason this is not a bare repository insert:
/// no account yet records the allow; a pending account is approved now,
/// exactly as `user approve` would, default MCP token included; an active
/// account is a no-op; a suspended account is left untouched, because
/// suspension is a deliberate admin act reversed explicitly, never as a
/// side effect of allowing an address. That holds for an account suspended
/// while the allow is being recorded too ([`approve_pending`]).
///
/// `caller` is who allows the address; the allow is recorded against
/// [`AccountCaller::recorded_as`]. Approving a pending account is acting on
/// it, so a pending admin's address takes the caller approving that admin
/// by id takes ([`AccountCaller::acting_on`]). The account is read, and
/// that rule applied, before the address is checked for its form, so an
/// address is refused for the account it names before anything else. An
/// address with no account, or whose account is not pending, changes no
/// account and is any caller's to allow.
///
/// # Errors
///
/// Returns a permission error for a pending admin's address and a caller
/// that may not act on an admin account, [`AppError::invalid_input`] for a
/// malformed address or an account whose standing changed twice under the
/// allow, or the repository's error when a lookup, the insert, or the
/// status write fails.
pub async fn allow(
    repos: &RepositoryRegistry,
    caller: &AccountCaller,
    email: &str,
    note: Option<&str>,
) -> AppResult<AllowResult> {
    // SECURITY: Global lookup — an allow names an address, which is in no tenant
    let account = repos.users.get_by_email(&normalize_email(email)).await?;
    let (pending, decided) = match account {
        Some(user) if user.user_status == UserStatus::Pending => {
            (Some(caller.acting_on(user)?), None)
        }
        Some(user) if user.user_status == UserStatus::Active => {
            (None, Some(AllowOutcome::AlreadyActive))
        }
        Some(_) => (None, Some(AllowOutcome::SuspendedUnchanged)),
        None => (None, None),
    };
    let email = valid_email(email)?;

    if let Some(acting) = pending {
        let (outcome, approved_user) = approve_pending(repos, &acting).await?;
        return Ok(AllowResult {
            email,
            outcome,
            approved_user,
        });
    }
    if let Some(outcome) = decided {
        return Ok(AllowResult {
            email,
            outcome,
            approved_user: None,
        });
    }

    let allowed_by = caller.recorded_as();
    let added = repos
        .pre_approved_emails
        .allow(&email, allowed_by, note)
        .await?;
    if added {
        info!(allowed_by = ?allowed_by, "Email pre-approved");
    }
    Ok(AllowResult {
        email,
        outcome: if added {
            AllowOutcome::Recorded
        } else {
            AllowOutcome::AlreadyAllowed
        },
        approved_user: None,
    })
}

/// Approve the account `acting` names, which the allow read as pending:
/// what the allow did, and the account to announce when it approved one.
///
/// The approval lands only while the account is still pending
/// ([`activate_pending_sign_up`]). An admin who decided the sign-up between
/// the allow's read and this write has the last word: the allow answers
/// what it would have answered had it read the account then, announces
/// nobody and provisions nothing.
async fn approve_pending(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
) -> AppResult<(AllowOutcome, Option<ApprovedUser>)> {
    let user = acting.account();
    let activation = activate_pending_sign_up(repos, acting).await?;
    if !activation.activated {
        return match activation.user.user_status {
            UserStatus::Active => Ok((AllowOutcome::AlreadyActive, None)),
            UserStatus::Suspended => Ok((AllowOutcome::SuspendedUnchanged, None)),
            UserStatus::Pending => Err(AppError::invalid_input(
                "This account's standing changed while the allow was recorded. Nothing was \
                 changed; allow the address again.",
            )),
        };
    }
    create_default_mcp_token_for_user(repos.user_mcp_tokens.as_ref(), user.id).await;
    info!(
        target_user_id = %user.id,
        "Pending user approved via pre-approval allow"
    );
    Ok((
        AllowOutcome::PendingApproved,
        Some(ApprovedUser {
            id: user.id,
            email: user.email.clone(),
            display_name: user.display_name.clone(),
        }),
    ))
}

/// Remove a standing pre-approval, and revoke the invitation link sent with it.
/// Never touches an existing account's status.
///
/// The invitation goes with the allow: redeeming one creates an active account,
/// which is exactly what an admin who withdraws the allow means to stop.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] for a malformed address, or the
/// repository's error when the delete fails.
pub async fn disallow(repos: &RepositoryRegistry, email: &str) -> AppResult<DisallowResult> {
    let email = valid_email(email)?;
    let removed = repos.pre_approved_emails.remove(&email).await?;
    if removed {
        info!("Pre-approved email removed");
    }
    let invitation_revoked = invitation::revoke(repos, &email).await?;
    let account_status = match repos.users.get_by_email(&email).await {
        Ok(user) => user.map(|u| u.user_status.to_string()),
        Err(e) => {
            warn!(error = %e, "Account lookup failed while removing a pre-approval");
            None
        }
    };
    Ok(DisallowResult {
        email,
        removed,
        invitation_revoked,
        account_status,
    })
}

/// List every standing pre-approval with its admin, registration state and
/// outstanding invitation.
///
/// # Errors
///
/// Returns the repository's error when the listing fails. A failed lookup of
/// one row's admin or account degrades that field to `None` rather than
/// failing the listing — an unreadable admin record is not a reason to
/// refuse to show the allow-list.
pub async fn list(repos: &RepositoryRegistry) -> AppResult<Vec<AllowedEmail>> {
    let entries = repos.pre_approved_emails.list().await?;
    let invitations = invitation::live_by_email(repos).await?;

    // One lookup per distinct admin, not per row: a cohort allowed in one
    // sitting shares an admin, and the listing is read often enough that
    // re-reading the same user row per entry is pure waste.
    let mut admin_emails: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut out = Vec::with_capacity(entries.len());

    for entry in entries {
        let allowed_by_email = admin_email(repos, &mut admin_emails, entry.allowed_by).await;

        let account_status = match repos.users.get_by_email(&entry.email).await {
            Ok(user) => user.map(|u| u.user_status.to_string()),
            Err(e) => {
                warn!(error = %e, "Account lookup failed for a pre-approval");
                None
            }
        };

        let invitation_expires_at = invitations
            .get(&entry.email)
            .map(|invitation| invitation.expires_at);
        out.push(AllowedEmail {
            email: entry.email,
            note: entry.note,
            created_at: entry.created_at,
            allowed_by: entry.allowed_by,
            allowed_by_email,
            account_status,
            invitation_expires_at,
        });
    }

    Ok(out)
}

/// Resolve one allow's admin email through a per-listing cache.
///
/// A failed lookup degrades to `None` rather than failing the listing: an
/// unreadable admin record is not a reason to refuse to show the
/// allow-list.
async fn admin_email(
    repos: &RepositoryRegistry,
    cache: &mut HashMap<Uuid, Option<String>>,
    allowed_by: Option<Uuid>,
) -> Option<String> {
    let id = allowed_by?;
    if let Some(cached) = cache.get(&id) {
        return cached.clone();
    }
    let resolved = match repos.users.get_global(id).await {
        Ok(user) => user.map(|u| u.email),
        Err(e) => {
            warn!(error = %e, "Admin lookup failed for a pre-approval");
            None
        }
    };
    cache.insert(id, resolved.clone());
    resolved
}
