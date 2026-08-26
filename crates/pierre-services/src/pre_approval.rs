// ABOUTME: Standing per-email pre-approval list — allow / disallow / list as one shared operation set
// ABOUTME: Both admin HTTP surfaces and pierre-cli reach the list through here, never the repository directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Operator management of the `pre_approved_emails` allow-list.
//!
//! An allow is standing: registration consults the list
//! ([`crate::auth::AuthService`]) so an allowed address lands
//! [`UserStatus::Active`] in one step with `approved_by` attributed to the
//! operator who recorded it, instead of queueing for approval.
//!
//! The three verbs live here rather than in a route handler because two
//! surfaces drive them — the bearer-token admin routes behind
//! `pierre-cli user allow / disallow / list-allowed`, and the cookie-authenticated
//! admin web app — and an allow must mean the same thing on both. In
//! particular, allowing an address that already has a *pending* account
//! approves it now (default MCP token included), which is the case an
//! operator hits whenever the person registered before being told not to.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserStatus;
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::admin_ops::{create_default_mcp_token_for_user, transition_user_status};
use crate::auth::AuthService;

/// Trim and lower-case an address, rejecting one that is not an email.
///
/// The table is keyed lower-case, so normalizing at the edge is what makes
/// `Allow` then `Disallow` of the same address with different capitalization
/// refer to one row.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when the value is not a valid email
/// address — rejected before it reaches the table, where a typo would sit as a
/// permanent allow nobody can match against.
pub fn normalize_email(email: &str) -> AppResult<String> {
    let normalized = email.trim().to_lowercase();
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

/// Outcome of [`allow`], carrying what the operator needs to be told and what
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
    /// Operator-facing sentence describing the outcome.
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
    /// Status of an account that exists for this address, if any. Removing a
    /// pre-approval never changes it, and an operator who expects otherwise
    /// needs to see that it did not.
    pub account_status: Option<String>,
}

impl DisallowResult {
    /// Operator-facing sentence describing the outcome.
    #[must_use]
    pub fn message(&self) -> String {
        let email = &self.email;
        let mut message = if self.removed {
            format!("{email} removed from the pre-approved list")
        } else {
            format!("{email} was not on the pre-approved list (nothing to remove)")
        };
        if let Some(status) = self.account_status.as_deref() {
            message.push_str(&format!(
                " — note: an account already exists for {email} (status: {status}), \
                 which disallow does not change"
            ));
        }
        message
    }
}

/// One pre-approved address with the state an operator reads it for.
#[derive(Debug, Clone, Serialize)]
pub struct AllowedEmail {
    /// The allowed address, lower-case.
    pub email: String,
    /// Operator note recorded with the allow (cohort, reason).
    pub note: Option<String>,
    /// When the allow was recorded.
    pub created_at: DateTime<Utc>,
    /// The operator who recorded it, when attributable.
    pub allowed_by: Option<Uuid>,
    /// That operator's email, resolved for display.
    pub allowed_by_email: Option<String>,
    /// Status of the account registered against this address, or `None` when
    /// nobody has registered yet — the whole point of a standing allow is that
    /// it outlives the wait, so "not yet" is a normal steady state.
    pub account_status: Option<String>,
}

/// Record a standing pre-approval, approving a pending account on the spot.
///
/// Four cases, and they are the reason this is not a bare repository insert:
/// no account yet records the allow; a pending account is approved now,
/// exactly as `user approve` would, default MCP token included; an active
/// account is a no-op; a suspended account is left untouched, because
/// suspension is a deliberate operator act reversed explicitly, never as a
/// side effect of allowing an address.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] for a malformed address, or the
/// repository's error when a lookup, the insert, or the status transition
/// fails.
pub async fn allow(
    repos: &RepositoryRegistry,
    email: &str,
    allowed_by: Option<Uuid>,
    note: Option<&str>,
) -> AppResult<AllowResult> {
    let email = normalize_email(email)?;

    if let Some(user) = repos.users.get_by_email(&email).await? {
        let (outcome, approved_user) = match user.user_status {
            UserStatus::Active => (AllowOutcome::AlreadyActive, None),
            UserStatus::Suspended => (AllowOutcome::SuspendedUnchanged, None),
            UserStatus::Pending => {
                transition_user_status(repos, user.id, UserStatus::Active, allowed_by).await?;
                create_default_mcp_token_for_user(repos.user_mcp_tokens.as_ref(), user.id).await;
                info!(
                    target_user_id = %user.id,
                    "Pending user approved via pre-approval allow"
                );
                (
                    AllowOutcome::PendingApproved,
                    Some(ApprovedUser {
                        id: user.id,
                        email: user.email.clone(),
                        display_name: user.display_name.clone(),
                    }),
                )
            }
        };
        return Ok(AllowResult {
            email,
            outcome,
            approved_user,
        });
    }

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

/// Remove a standing pre-approval. Never touches an existing account's status.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] for a malformed address, or the
/// repository's error when the delete fails.
pub async fn disallow(repos: &RepositoryRegistry, email: &str) -> AppResult<DisallowResult> {
    let email = normalize_email(email)?;
    let removed = repos.pre_approved_emails.remove(&email).await?;
    if removed {
        info!("Pre-approved email removed");
    }
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
        account_status,
    })
}

/// List every standing pre-approval with its operator and registration state.
///
/// # Errors
///
/// Returns the repository's error when the listing fails. A failed lookup of
/// one row's operator or account degrades that field to `None` rather than
/// failing the listing — an unreadable operator record is not a reason to
/// refuse to show the allow-list.
pub async fn list(repos: &RepositoryRegistry) -> AppResult<Vec<AllowedEmail>> {
    let entries = repos.pre_approved_emails.list().await?;

    // One lookup per distinct operator, not per row: a cohort allowed in one
    // sitting shares an operator, and the listing is read often enough that
    // re-reading the same user row per entry is pure waste.
    let mut operator_emails: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut out = Vec::with_capacity(entries.len());

    for entry in entries {
        let allowed_by_email = operator_email(repos, &mut operator_emails, entry.allowed_by).await;

        let account_status = match repos.users.get_by_email(&entry.email).await {
            Ok(user) => user.map(|u| u.user_status.to_string()),
            Err(e) => {
                warn!(error = %e, "Account lookup failed for a pre-approval");
                None
            }
        };

        out.push(AllowedEmail {
            email: entry.email,
            note: entry.note,
            created_at: entry.created_at,
            allowed_by: entry.allowed_by,
            allowed_by_email,
            account_status,
        });
    }

    Ok(out)
}

/// Resolve one allow's operator email through a per-listing cache.
///
/// A failed lookup degrades to `None` rather than failing the listing: an
/// unreadable operator record is not a reason to refuse to show the
/// allow-list.
async fn operator_email(
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
            warn!(error = %e, "Operator lookup failed for a pre-approval");
            None
        }
    };
    cache.insert(id, resolved.clone());
    resolved
}
