// ABOUTME: Withdraws the credentials an account held on its admin standing when it stops being an active admin
// ABOUTME: Device-login tokens it approved and impersonations it opened; shared by the console, the admin API and pierre-cli
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An admin's power leaves the account in two credentials that are not the
//! account's own session: the non-expiring service token a device login
//! mints (`admin_tokens.operator_user_id` names the admin who approved it),
//! and the one-hour token of an impersonation the admin opened.
//!
//! Both are refused on their next request once the admin is demoted,
//! suspended, set back to pending or deleted, because the gates that take
//! them read the admin's row (`AdminAuthService::authenticate`, and the
//! impersonation check of the auth middleware). What makes the loss
//! permanent is the write itself: the repository deactivates the tokens and
//! ends the sessions in the transaction that demotes or deactivates the
//! account (`UserRepository::set_admin_status`, `update_status`, `update`),
//! so no write through the repository can leave them live and a later
//! promotion or approval does not bring them back. A row changed past the
//! repository, by hand or by another revision's SQL, is covered by the gates
//! alone. A deleted account's device logins end with it, in the deletion's
//! own transaction (`AccountDeletionRepository::delete_account`): their rows
//! stay, inactive and acting as no account, so what they did stays on
//! record. Its impersonations go with its row. This module puts what was
//! taken away on the trail.

pub use pierre_database::repositories::WithdrawnAdminCredentials;
use pierre_database::repositories::{AuditOutcome, AuditSource, NewAuditEvent};
use pierre_database::RepositoryRegistry;
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

/// The action key of the trail row a withdrawal writes.
pub const ADMIN_CREDENTIALS_REVOKED: &str = "admin.credentials.revoke";

/// How an account stopped being an active admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminStandingLost {
    /// Its role was set back to `user`.
    Demoted,
    /// It was moved out of the active status: suspended, or set back to
    /// pending.
    Deactivated,
    /// The account was deleted.
    Deleted,
}

impl AdminStandingLost {
    /// The word the trail uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Demoted => "demoted",
            Self::Deactivated => "deactivated",
            Self::Deleted => "deleted",
        }
    }
}

/// Who took the account out of standing, and through what, for the trail.
#[derive(Debug, Clone, Copy)]
pub struct StandingChange {
    /// How the account stopped being an active admin.
    pub cause: AdminStandingLost,
    /// The account, while it exists. `None` for a deleted one: the trail
    /// keeps neither its id nor the ids of the tokens that named it.
    pub account: Option<Uuid>,
    /// The admin who made the change, when the credential names one.
    pub actor: Option<Uuid>,
    /// The surface the change came through.
    pub source: AuditSource,
}

/// Write the trail row for `withdrawn`; nothing when it is empty.
///
/// The caller is the surface that made the change, since only it knows who
/// acted and through what. A failed write is logged and never fails the
/// change it describes, as every other row of the trail.
pub async fn record_withdrawn_admin_credentials(
    repos: &RepositoryRegistry,
    withdrawn: &WithdrawnAdminCredentials,
    change: StandingChange,
) {
    if withdrawn.is_empty() {
        return;
    }
    let cause = change.cause.as_str();
    let mut metadata = json!({
        "cause": cause,
        "device_tokens_revoked": withdrawn.device_tokens.len(),
        "impersonations_ended": withdrawn.impersonations,
    });
    if let Some(account) = change.account {
        metadata["target"] = json!({ "user_id": account });
        metadata["device_token_ids"] = json!(withdrawn.device_tokens);
    }
    let event = NewAuditEvent {
        action: ADMIN_CREDENTIALS_REVOKED.to_owned(),
        summary: format!(
            "Revoked {} device login(s) and ended {} impersonation(s) of a {cause} admin",
            withdrawn.device_tokens.len(),
            withdrawn.impersonations,
        ),
        outcome: AuditOutcome::Success,
        source: change.source,
        actor_user_id: change.actor,
        tenant_id: None,
        ip_address: None,
        user_agent: None,
        metadata,
    };
    if let Err(e) = repos.admin_audit.record(&event).await {
        warn!(error = %e, action = ADMIN_CREDENTIALS_REVOKED, "Failed to record withdrawn admin credentials in the audit trail");
    }
}
