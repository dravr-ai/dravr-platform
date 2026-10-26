// ABOUTME: Signup codes on AuthService: register with a code, redeem one from the pending page, activate once the address is proven
// ABOUTME: A code approves the account in its minter's name, never past a closed kill switch, a deleted account's hold or a suspension (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How a signup code turns into an approved account.
//!
//! Entering a code claims one of its uses and records it against the account
//! ([`AuthService::redeem_signup_code`]). Whether the account is activated
//! now depends on its address: a code vouches for the cohort, not for the
//! inbox, so an address not yet proven (a password sign-up before its
//! confirmation link) leaves the account Pending with the redemption waiting,
//! and proving it activates the account through
//! [`AuthService::code_awaiting_activation`]. A Google sign-in, or an address
//! already confirmed, activates at once.

use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use tracing::{info, warn};
use uuid::Uuid;

use pierre_auth::dto::auth::{RegisterRequest, RegisterResponse, RegistrationActivation};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::GroupInvite;
use pierre_core::models::{SignupCodeRedemption, SignupCodeSource, User, UserStatus};

use super::AuthService;
use crate::admin_ops::{create_default_mcp_token_for_user, transition_user_status};
use crate::import_tombstones::email_held;
use crate::signup_codes::{self, invalid_signup_code};
use crate::signup_policy::admit_signup_code;

/// What redeeming a code did to the account.
#[derive(Debug, Clone)]
pub struct RedeemedSignupCode {
    /// What was redeemed
    pub source: SignupCodeSource,
    /// The account's status now: Active when the code activated it, Pending
    /// while it waits on the address
    pub status: UserStatus,
    /// The coach's group invite to join, for a coach's code. Joining belongs
    /// to the caller, which holds the group service.
    pub coach_invite: Option<GroupInvite>,
}

impl AuthService {
    /// Register through a stranger's door with a signup code.
    ///
    /// The code is checked before anything is written, so a mistyped one is
    /// answered while the person can still fix it. A valid code admits the
    /// sign-up past the daily ceiling but not past a closed kill switch.
    /// The account is then created as any self-serve one is and the code is
    /// redeemed against it; a code that was used up between the check and
    /// the claim leaves the account Pending rather than failing it. An
    /// account registration already made active (an operator allowed its
    /// address) still spends the code, so a coach's code joins its group; a
    /// claim that fails there leaves the account active and in no group.
    ///
    /// # Errors
    ///
    /// Returns the uniform invalid-code error for a code no one could redeem,
    /// a quota-exceeded error when sign-up is closed, and every error
    /// [`Self::register`] returns.
    pub async fn self_register_with_code(
        &self,
        request: RegisterRequest,
        code: &SecretString,
    ) -> AppResult<(RegisterResponse, Option<RedeemedSignupCode>)> {
        let repos = self.data.repos();
        if !signup_codes::is_live(repos, code.expose_secret()).await? {
            return Err(invalid_signup_code());
        }
        admit_signup_code(self.data.database()).await?;

        let mut response = self.register_through(request, true, true).await?;
        let user_id = Uuid::parse_str(&response.user_id)
            .map_err(|e| AppError::internal(format!("Registration returned a bad id: {e}")))?;
        if response.user_status == UserStatus::Active {
            // An operator's allow already activated the account; the code
            // still carries the coach's group, so it is spent and joined.
            return match self.redeem_for_active_signup(user_id, code).await {
                Ok(redeemed) => Ok((response, Some(redeemed))),
                Err(e) => {
                    warn!(user_id = %user_id, error = %e, "Signup code not redeemed for an account its allow activated; it joins no group");
                    Ok((response, None))
                }
            };
        }
        let redeemed = match self.redeem_signup_code(user_id, code).await {
            Ok(redeemed) => redeemed,
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Signup code could not be redeemed after registration; the account waits for review");
                self.review_without_code(user_id, &mut response).await;
                return Ok((response, None));
            }
        };
        response.user_status = redeemed.status;
        response.activation = if redeemed.status == UserStatus::Active {
            RegistrationActivation::Active
        } else {
            RegistrationActivation::ConfirmEmail
        };
        response
            .activation
            .message()
            .clone_into(&mut response.message);
        Ok((response, Some(redeemed)))
    }

    /// The code checked live before registering could not be redeemed (used
    /// up or revoked in between), so no confirmation link will activate the
    /// account. Registering skipped the review announcement only because the
    /// code vouched for it; unless a proven address activates it anyway, it
    /// waits for an operator like any account no rule admitted: it is told
    /// so, and announced for review. A deleted account's address was never
    /// vouched for (registering announced it already), so it is left alone.
    async fn review_without_code(&self, user_id: Uuid, response: &mut RegisterResponse) {
        let repos = self.data.repos();
        let user = match repos.users.get_global(user_id).await {
            Ok(Some(user)) => user,
            Ok(None) => return,
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Could not read the account a lost signup code left pending");
                return;
            }
        };
        if user.user_status != UserStatus::Pending
            || email_held(repos, &user.email).await
            || self.activates_on_verified(&user.email).await
        {
            return;
        }
        response.activation = RegistrationActivation::Review;
        RegistrationActivation::Review
            .message()
            .clone_into(&mut response.message);
        match repos.tenants.list_for_user(user_id).await {
            Ok(tenants) => {
                if let Some(tenant) = tenants.first() {
                    Self::announce_if_pending(&user, tenant.id, false);
                }
            }
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Could not read the tenant to announce a pending account");
            }
        }
    }

    /// Redeem a signup code for a Pending account: claim one use, record it,
    /// and activate the account now when its address is proven.
    ///
    /// A code never overrides a deleted account's hold on the address, never
    /// revives a suspended account, and never gets past a closed kill switch.
    /// An account holds one redemption.
    ///
    /// # Errors
    ///
    /// Returns a not-found error for an unknown account, a suspended-account
    /// error, an invalid-input error for an account that is already active,
    /// holds a deleted account's address or already redeemed a code, the
    /// uniform invalid-code error, a quota-exceeded error when sign-up is
    /// closed, and the repository's error when a write fails.
    pub async fn redeem_signup_code(
        &self,
        user_id: Uuid,
        code: &SecretString,
    ) -> AppResult<RedeemedSignupCode> {
        let repos = self.data.repos();
        let user = repos
            .users
            .get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found("User not found"))?;
        match user.user_status {
            UserStatus::Suspended => {
                return Err(AppError::account_suspended(
                    "Your account has been suspended",
                ))
            }
            UserStatus::Active => {
                return Err(AppError::invalid_input("Your account is already active"))
            }
            UserStatus::Pending => {}
        }
        if email_held(repos, &user.email).await {
            return Err(AppError::invalid_input(
                "This address belonged to a deleted Dravr account. Start fresh with it first, then enter your code.",
            ));
        }
        if repos.signup_codes.redemption_for(user_id).await?.is_some() {
            return Err(AppError::invalid_input(
                "This account already used a signup code",
            ));
        }
        admit_signup_code(self.data.database()).await?;

        let claimed = self.claim_and_record(user_id, code).await?;

        let status = if self.email_already_proven(&user).await? {
            self.activate_by_code(&user, claimed.granted_by).await?;
            UserStatus::Active
        } else {
            UserStatus::Pending
        };
        Ok(RedeemedSignupCode {
            source: claimed.source,
            status,
            coach_invite: claimed.coach_invite,
        })
    }

    /// Redeem a code for an account registration has just made active (an
    /// operator allowed its address): one use is claimed and recorded, and the
    /// redemption is marked activated so no later confirmation reapplies it.
    /// The account's approver is left as the allow set it.
    async fn redeem_for_active_signup(
        &self,
        user_id: Uuid,
        code: &SecretString,
    ) -> AppResult<RedeemedSignupCode> {
        let claimed = self.claim_and_record(user_id, code).await?;
        self.data
            .repos()
            .signup_codes
            .mark_activated(user_id, Utc::now())
            .await?;
        Ok(RedeemedSignupCode {
            source: claimed.source,
            status: UserStatus::Active,
            coach_invite: claimed.coach_invite,
        })
    }

    /// Claim one use of `code` and record it against `user_id`, releasing the
    /// use again when the account already holds a redemption.
    async fn claim_and_record(
        &self,
        user_id: Uuid,
        code: &SecretString,
    ) -> AppResult<signup_codes::ClaimedCode> {
        let repos = self.data.repos();
        let claimed = signup_codes::claim(repos, code.expose_secret())
            .await?
            .ok_or_else(invalid_signup_code)?;
        let recorded = repos
            .signup_codes
            .record_redemption(user_id, claimed.source, claimed.granted_by, Utc::now())
            .await?;
        if !recorded {
            signup_codes::release(repos, &claimed).await;
            return Err(AppError::invalid_input(
                "This account already used a signup code",
            ));
        }
        info!(user_id = %user_id, source = ?claimed.source, "Signup code redeemed");
        Ok(claimed)
    }

    /// The redemption waiting to activate `user`: one is recorded, it has not
    /// activated the account yet, and the account's address is now proven.
    /// A read that fails counts as none, so a hiccup never activates anyone.
    pub(super) async fn code_awaiting_activation(
        &self,
        user: &User,
    ) -> Option<SignupCodeRedemption> {
        let redemption = match self.data.repos().signup_codes.redemption_for(user.id).await {
            Ok(Some(redemption)) if redemption.activated_at.is_none() => redemption,
            Ok(_) => return None,
            Err(e) => {
                warn!(user_id = %user.id, error = %e, "Could not read the account's signup code; it stays pending");
                return None;
            }
        };
        match self.email_already_proven(user).await {
            Ok(true) => Some(redemption),
            Ok(false) => None,
            Err(e) => {
                warn!(user_id = %user.id, error = %e, "Could not read the verification state; it stays pending");
                None
            }
        }
    }

    /// Activate `user` on a code: Active in the minter's name while that
    /// account exists, its default MCP token, and the redemption marked.
    pub(super) async fn activate_by_code(
        &self,
        user: &User,
        granted_by: Option<Uuid>,
    ) -> AppResult<()> {
        let repos = self.data.repos();
        let approver = self.existing_operator(granted_by).await;
        transition_user_status(repos, user.id, UserStatus::Active, approver).await?;
        create_default_mcp_token_for_user(repos.user_mcp_tokens.as_ref(), user.id).await;
        repos
            .signup_codes
            .mark_activated(user.id, Utc::now())
            .await?;
        info!(user_id = %user.id, approved_by = ?approver, "Account activated by a signup code");
        Ok(())
    }
}
