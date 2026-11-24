// ABOUTME: Self-serve sign-up on AuthService: the admin's bounds on a stranger's door, activation on a proven address
// ABOUTME: Split from auth.rs; the bounds themselves live in crate::signup_policy (carnet#1063)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The account rules a stranger's sign-up meets on top of a registration:
//! the kill switch and the 24-hour ceiling before anything is written, and,
//! with sign-up open to verified addresses, proving the address as what
//! activates the account.

use tracing::{info, warn};

use pierre_auth::dto::auth::{RegisterRequest, RegisterResponse, RegistrationActivation};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{User, UserStatus};

use super::AuthService;
use crate::admin_ops::{activate_pending_sign_up, AccountCaller};
use crate::import_tombstones::email_held;
use crate::signup_policy::{admit_self_signup, resolve_signup_policy};

impl AuthService {
    /// Register an account through a door any stranger can reach: the public
    /// form, the messaging link page, in-chat sign-up.
    ///
    /// The admin's bounds apply first ([`admit_self_signup`]): a closed
    /// kill switch or a reached 24-hour ceiling refuses before anything is
    /// validated or written. Then the account is registered and may
    /// activate itself by proving its address.
    ///
    /// # Errors
    ///
    /// Returns a quota-exceeded error when sign-up is paused, and every
    /// error registration returns (invalid email, weak password, an
    /// existing account).
    pub async fn self_register(&self, request: RegisterRequest) -> AppResult<RegisterResponse> {
        self.admit_self_signup().await?;
        self.register_through(request, false).await
    }

    /// Refuse a self-serve sign-up the admin's bounds do not admit
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
    /// account's email waits for an admin or an explicit clear.
    pub(super) async fn activates_on_verified(&self, email: &str) -> bool {
        self.activates_on_verified_email().await && !email_held(self.data.repos(), email).await
    }

    /// Whether a sign-up rule may activate `user` without an admin deciding:
    /// it is still pending, no deleted account's tombstone holds its
    /// address, and it does not hold the admin role.
    ///
    /// An admin is approved by an admin (carnet#1589). A pending account
    /// that holds the role (one set back to pending, or given the role by a
    /// write that did not check its status) would otherwise hold every
    /// power the moment an open sign-up rule, a standing allow or a signup
    /// code let it in. It waits for an approval like any account no rule
    /// admits.
    pub(super) async fn may_self_activate(&self, user: &User) -> bool {
        user.user_status == UserStatus::Pending
            && !user.role.is_admin()
            && !email_held(self.data.repos(), &user.email).await
    }

    /// Re-evaluate auto-approval for existing Pending users.
    ///
    /// When `AUTO_APPROVE_DOMAINS` gains a domain — or an admin allow-lists
    /// the exact address — after a user registered, their account stays
    /// Pending forever. This method promotes them to Active on their next
    /// login if their email now qualifies, attributing `approved_by` to the
    /// allow-listing admin when the promotion comes from a per-email
    /// pre-approval.
    ///
    /// With sign-up open to verified addresses, a proven address qualifies
    /// too ([`Self::verified_and_self_activating`]); an unproven one never
    /// does, at login or anywhere else. Nor does one a deleted account's
    /// tombstone holds, until it is explicitly cleared, nor an account that
    /// holds the admin role, ever ([`Self::may_self_activate`]).
    ///
    /// `user` is the caller's earlier read, and the promotion lands only
    /// while the stored account is still pending
    /// ([`activate_pending_sign_up`]). Once a rule admits the account,
    /// `user` leaves with the status the store holds, whether or not the
    /// promotion landed: a caller that signs the account in refuses one
    /// suspended since its read (`Self::reject_if_suspended`). An account no
    /// rule admits is not read again, and leaves as the caller read it.
    pub(super) async fn auto_approve_if_eligible(&self, user: &mut User) -> AppResult<()> {
        if !self.may_self_activate(user).await {
            return Ok(());
        }

        // A signup code entered before the address was proven (carnet#1128).
        if let Some(redemption) = self.code_awaiting_activation(user).await {
            let stored = self.activate_by_code(user, redemption.granted_by).await?;
            user.user_status = stored.user_status;
            user.approved_at = stored.approved_at;
            return Ok(());
        }

        if !self.should_auto_approve_email(&user.email).await?
            && !self.verified_and_self_activating(user.id).await
        {
            return Ok(());
        }

        let allowed_by = self
            .pre_approval_for(&user.email)
            .await
            .and_then(|entry| entry.allowed_by);
        let rule = AccountCaller::sign_up_rule(self.existing_admin(allowed_by).await);
        let activation =
            activate_pending_sign_up(self.data.repos(), &rule.acting_on(user.clone())?).await?;
        // Said only of an activation that landed; one that did not is
        // logged where it was found decided.
        if activation.activated {
            info!(user_id = %user.id, "Retroactive auto-approval for pending user");
        }
        user.user_status = activation.user.user_status;
        user.approved_at = activation.user.approved_at;

        Ok(())
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

    /// Whether `user`, still Pending, waits only on proving its address: no
    /// admin is asked to review it, so the next step is the confirmation
    /// link, not an approval. That holds while sign-up is open to verified
    /// addresses (and no tombstone holds the address), or when a signup code
    /// it redeemed waits for the proof (carnet#1128). A state that cannot be
    /// read counts as not waiting on the address, so the athlete is told the
    /// account is in review rather than sent after a link that activates
    /// nothing.
    pub async fn awaits_email_proof(&self, user: &User) -> bool {
        // An admin's account waits on an admin's approval, whatever it
        // proves ([`Self::may_self_activate`]).
        if user.user_status != UserStatus::Pending || user.role.is_admin() {
            return false;
        }
        match self.email_already_proven(user).await {
            Ok(false) => {}
            Ok(true) => return false,
            Err(e) => {
                warn!(user_id = %user.id, error = %e, "Could not read the verification state; reporting the account as in review");
                return false;
            }
        }
        if self.activates_on_verified(&user.email).await {
            return true;
        }
        match self.data.repos().signup_codes.redemption_for(user.id).await {
            Ok(redemption) => redemption.is_some_and(|r| r.activated_at.is_none()),
            Err(e) => {
                warn!(user_id = %user.id, error = %e, "Could not read the account's signup code; reporting the account as in review");
                false
            }
        }
    }

    /// What `user` waits on, as a registration would have answered: `Active`
    /// for an active account; for a Pending one `ConfirmEmail` while it waits
    /// only on proving its address ([`Self::awaits_email_proof`]), else
    /// `Review`. `None` for a suspended account, which waits on nothing.
    ///
    /// Nothing about self-activation is stored on the account, so this
    /// re-derives it from the inputs registration used: the sign-up policy,
    /// the address's tombstone, a signup code waiting on the proof, and
    /// whether the address is already proven.
    pub async fn activation_of(&self, user: &User) -> Option<RegistrationActivation> {
        match user.user_status {
            UserStatus::Active => Some(RegistrationActivation::Active),
            UserStatus::Pending => Some(if self.awaits_email_proof(user).await {
                RegistrationActivation::ConfirmEmail
            } else {
                RegistrationActivation::Review
            }),
            UserStatus::Suspended => None,
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
    /// waits, the admins are emailed.
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
