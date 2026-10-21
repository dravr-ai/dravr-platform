// ABOUTME: Self-serve sign-up on AuthService: the operator's bounds on a stranger's door, activation on a proven address
// ABOUTME: Split from auth.rs; the bounds themselves live in crate::signup_policy (carnet#1063)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The account rules a stranger's sign-up meets on top of a registration:
//! the kill switch and the 24-hour ceiling before anything is written, and,
//! with sign-up open to verified addresses, proving the address as what
//! activates the account.

use tracing::warn;

use pierre_auth::dto::auth::{RegisterRequest, RegisterResponse};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserStatus;

use super::AuthService;
use crate::import_tombstones::email_held;
use crate::signup_policy::{admit_self_signup, resolve_signup_policy};

impl AuthService {
    /// Register an account through a door any stranger can reach: the public
    /// form, the messaging link page, in-chat sign-up.
    ///
    /// The operator's bounds apply first ([`admit_self_signup`]): a closed
    /// kill switch or a reached 24-hour ceiling refuses before anything is
    /// validated or written. Then it is [`Self::register`], as a self-serve
    /// account.
    ///
    /// # Errors
    ///
    /// Returns a quota-exceeded error when sign-up is paused, and every
    /// error [`Self::register`] returns.
    pub async fn self_register(&self, request: RegisterRequest) -> AppResult<RegisterResponse> {
        self.admit_self_signup().await?;
        self.register_through(request, true).await
    }

    /// Refuse a self-serve sign-up the operator's bounds do not admit
    /// ([`admit_self_signup`]).
    pub(super) async fn admit_self_signup(&self) -> AppResult<()> {
        admit_self_signup(self.data.database(), self.data.repos().users.as_ref()).await
    }

    /// Whether proving the address activates an account no auto-approval rule
    /// admitted: sign-up open to verified addresses (carnet#1063).
    pub(super) async fn activates_on_verified_email(&self) -> bool {
        resolve_signup_policy(self.data.database())
            .await
            .activate_on_verified_email
    }

    /// Whether proving `email` activates its new account: sign-up is open to
    /// verified addresses and no tombstone holds the address. A deleted
    /// account's email waits for an operator or an explicit clear.
    pub(super) async fn activates_on_verified(&self, email: &str) -> bool {
        self.activates_on_verified_email().await && !email_held(self.data.repos(), email).await
    }

    /// Whether `user_id` has proven their address while sign-up is open to
    /// verified addresses. A verification state that cannot be read counts as
    /// unproven, so a database hiccup never activates an account.
    pub(super) async fn verified_and_self_activating(&self, user_id: uuid::Uuid) -> bool {
        if !self.activates_on_verified_email().await {
            return false;
        }
        match self
            .data
            .repos()
            .email_verification
            .is_verified(user_id)
            .await
        {
            Ok(verified) => verified,
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Could not read the verification state; the account stays pending");
                false
            }
        }
    }

    /// Apply the approval decision to a user who has just proven their address.
    ///
    /// Verification and approval are separate gates on purpose. Confirming an
    /// address proves the person owns the inbox; it does not decide whether this
    /// deployment lets them in. So a verified user is promoted to `Active` only
    /// when the auto-approval decision already says yes — the global switch, a
    /// per-email pre-approval, or a domain on the allow-list.
    ///
    /// That keeps one code path working in both postures. While the deployment is
    /// invite-only the user stays `Pending`, but *verified* pending, so the waiting
    /// screen can say the address is confirmed and review is what's outstanding.
    /// Open sign-up to verified addresses (`signup_activate_on_verified_email`)
    /// and the same call promotes them immediately, with no second
    /// implementation for "open signup".
    ///
    /// `first_proof` is what the repository's `mark_verified`
    /// returned: when this proof is the address's first and the account still
    /// waits, the super-admins are emailed.
    ///
    /// Returns the status the user now holds.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cannot be loaded or the status write fails.
    pub async fn apply_approval_after_verification(
        &self,
        user_id: uuid::Uuid,
        first_proof: bool,
    ) -> AppResult<UserStatus> {
        let Some(mut user) = self.data.repos().users.get_global(user_id).await? else {
            return Err(AppError::not_found("User not found"));
        };

        // Suspended accounts are never revived by confirming an email.
        if user.user_status != UserStatus::Pending {
            return Ok(user.user_status);
        }

        self.auto_approve_if_eligible(&mut user).await?;
        if first_proof {
            self.mail_admins_if_pending(&user);
        }
        Ok(user.user_status)
    }
}
