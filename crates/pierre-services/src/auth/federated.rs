// ABOUTME: Sign-in through an external identity — Firebase in the web app, Google on the hosted OAuth login page
// ABOUTME: One set of account rules: find by identity, attach only on a proven email, create through the signup posture
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every federated sign-in resolves its account the same way, whichever path
//! proved the identity:
//!
//! 1. the account its Firebase UID is linked to (Firebase sign-ins only);
//! 2. the account its Google account id (`sub`) is linked to;
//! 3. the account its email names, attached only when the sign-in proves
//!    that email ([`FederatedIdentity::proves_email`]) and the account is not
//!    already linked to another Google account;
//! 4. otherwise a new account, under the same approval posture as a
//!    registration, with a personal workspace.
//!
//! Attaching to an account whose email was never verified retires every
//! credential it held before: whoever registered that address first proved
//! nothing about it, so their password and sessions end here.

use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

use pierre_auth::dto::auth::LoginResponse;
use pierre_auth::google_oidc::GOOGLE_PROVIDER;
use pierre_core::constants::{limits, tiers};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    default_locale, normalize_email, CoachingPersona, User, UserTier, FEDERATED_ONLY_PASSWORD_HASH,
};
use pierre_core::permissions::UserRole;
use pierre_middleware::mask_email;

use super::{invalid_credentials, AuthService};

/// Where a federated account was created, as the `user.signed_up` notify
/// event's `source` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignupSource {
    /// Firebase sign-in in the web or mobile app
    Firebase,
    /// Google sign-in on the hosted OAuth authorization-server login page
    Google,
}

impl SignupSource {
    /// The `source` value the notify event carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Firebase => "firebase",
            Self::Google => "google",
        }
    }
}

/// An identity an external provider proved, as the account rules read it.
#[derive(Debug, Clone)]
pub struct FederatedIdentity<'a> {
    /// The Firebase UID (Firebase sign-ins only); never a Google `sub`
    pub firebase_uid: Option<&'a str>,
    /// The Google account id (`sub`), from either path
    pub google_subject: Option<&'a str>,
    /// The email the identity carries, normalized
    pub email: String,
    /// Whether the provider verified that email
    pub email_verified: bool,
    /// The display name a new account starts with
    pub display_name: Option<&'a str>,
    /// The sign-in provider (`google.com`, `apple.com`, ...), recorded as the
    /// account's `auth_provider`
    pub provider: &'a str,
    /// What the signup event reports as the account's source
    pub signup_source: SignupSource,
}

impl FederatedIdentity<'_> {
    /// Whether this sign-in proves `account_email`: the provider verified the
    /// address it carries, and it is `account_email` once both are
    /// normalized. The web app's Firebase sign-in and the hosted Google
    /// sign-in apply this one rule. Anything less proves nothing about who
    /// holds the account's address.
    fn proves_email(&self, account_email: &str) -> bool {
        self.email_verified && self.email == normalize_email(account_email)
    }
}

impl AuthService {
    /// Sign in the person an external provider proved, by the rules this
    /// module documents, and mint their session.
    ///
    /// A suspended account is refused; a pending one signs in, and the
    /// response carries its status. Raises `user.login`, and `user.signed_up`
    /// when the account is new.
    ///
    /// # Errors
    ///
    /// The generic credentials refusal when the identity's email names an
    /// account the sign-in cannot prove it holds, or one linked to another
    /// Google account; `account_suspended` for a suspended account; the
    /// repository error when a read or write fails.
    pub async fn login_with_federated_identity(
        &self,
        identity: FederatedIdentity<'_>,
    ) -> AppResult<LoginResponse> {
        let mut user = self.find_or_create_federated_user(&identity).await?;

        // Block suspended users; pending users authenticate so the frontend
        // can show the "pending approval" page (user_status is in the response).
        Self::reject_if_suspended(&user)?;

        self.record_federated_email_proof(&user, &identity).await?;

        // Retroactively approve pending users whose domain now qualifies
        self.auto_approve_if_eligible(&mut user).await?;

        let response = self
            .complete_federated_login(&user, identity.provider)
            .await?;

        // Every successful auth raises user.login. tenant_id is optional on
        // UserInfo — emit an empty field rather than a literal "None" when it
        // is absent, mirroring how the OAuth2 token handler records it.
        info!(
            target: "notify",
            event = "user.login",
            user_id = %user.id,
            tenant_id = %response.user.tenant_id.as_deref().unwrap_or_default(),
            "user authenticated"
        );

        Ok(response)
    }

    /// Mark `user`'s email verified when the sign-in proves it
    /// ([`FederatedIdentity::proves_email`]), as the confirmation link does.
    /// A sign-in never vouches for an address the account does not hold, and
    /// one that proves nothing leaves an earlier mark as it is.
    async fn record_federated_email_proof(
        &self,
        user: &User,
        identity: &FederatedIdentity<'_>,
    ) -> AppResult<()> {
        if !identity.proves_email(&user.email) {
            return Ok(());
        }
        self.data
            .repos()
            .email_verification
            .mark_verified(user.id)
            .await?;
        info!(user_id = %user.id, provider = %identity.provider, "Email verified by the sign-in provider");
        Ok(())
    }

    /// Find the account `identity` signs in to, or create it (the module
    /// documentation lists the order), and link its Google account id.
    ///
    /// # Errors
    ///
    /// Returns [`invalid_credentials`] for an email naming an existing
    /// account the sign-in cannot attach to, and the repository error when a
    /// read or write fails.
    async fn find_or_create_federated_user(
        &self,
        identity: &FederatedIdentity<'_>,
    ) -> AppResult<User> {
        let (by_uid, by_subject) = self.linked_accounts(identity).await?;

        if let Some(user) = by_uid {
            if by_subject.is_some_and(|linked| linked != user.id) {
                warn!(
                    user_id = %user.id,
                    "Firebase UID and Google account id name different accounts; the Firebase UID wins"
                );
            }
            self.link_google_subject(user.id, identity).await?;
            return Ok(user);
        }

        let repos = self.data.repos();
        if let Some(user_id) = by_subject {
            return repos
                .users
                .get_global(user_id)
                .await?
                .ok_or_else(|| AppError::not_found("User"));
        }

        if let Some(user) = repos.users.get_by_email(&identity.email).await? {
            return self.attach_federated_user(user, identity).await;
        }

        let user = self.create_federated_user(identity).await?;
        self.link_google_subject(user.id, identity).await?;
        Ok(user)
    }

    /// The account the identity's Firebase UID names, and the account id its
    /// Google account id is linked to.
    async fn linked_accounts(
        &self,
        identity: &FederatedIdentity<'_>,
    ) -> AppResult<(Option<User>, Option<Uuid>)> {
        let repos = self.data.repos();
        let by_uid = match identity.firebase_uid {
            Some(uid) => repos.users.get_by_firebase_uid(uid).await?,
            None => None,
        };
        let by_subject = match identity.google_subject {
            Some(subject) => {
                repos
                    .federated_identities
                    .user_for_subject(GOOGLE_PROVIDER, subject)
                    .await?
            }
            None => None,
        };
        Ok((by_uid, by_subject))
    }

    /// Attach `identity` to the existing account `user` its email found,
    /// once [`Self::may_attach`] allows it, retiring the account's earlier
    /// credentials when its email was never verified.
    async fn attach_federated_user(
        &self,
        mut user: User,
        identity: &FederatedIdentity<'_>,
    ) -> AppResult<User> {
        self.may_attach(&user, identity).await?;

        let repos = self.data.repos();
        if !self.email_already_proven(&user).await? {
            self.retire_unproven_credentials(&mut user, identity)
                .await?;
        }

        info!(user_id = %user.id, provider = %identity.provider, "Linking existing account to the federated identity");
        if let Some(uid) = identity.firebase_uid {
            user.firebase_uid = Some(uid.to_owned());
        }
        identity.provider.clone_into(&mut user.auth_provider);
        // `update`, not `create`: the row exists.
        repos.users.update(&user).await?;
        self.link_google_subject(user.id, identity).await?;
        Ok(user)
    }

    /// Whether `identity` may attach to `user`: it proves the account's
    /// email, and the account is not linked to another Google account (an
    /// address reassigned to a new Google account). A refusal is the generic
    /// [`invalid_credentials`], which does not say the account exists.
    async fn may_attach(&self, user: &User, identity: &FederatedIdentity<'_>) -> AppResult<()> {
        if !identity.proves_email(&user.email) {
            warn!(
                email = %mask_email(&user.email),
                provider = %identity.provider,
                "Federated sign-in refused: an unproven email names an existing account"
            );
            return Err(invalid_credentials());
        }
        let Some(subject) = identity.google_subject else {
            return Ok(());
        };
        let linked = self
            .data
            .repos()
            .federated_identities
            .subject_for_user(user.id, GOOGLE_PROVIDER)
            .await?;
        if linked.is_some_and(|linked| linked != subject) {
            warn!(
                user_id = %user.id,
                provider = %identity.provider,
                "Federated sign-in refused: the email's account is linked to another Google account"
            );
            return Err(invalid_credentials());
        }
        Ok(())
    }

    /// Whether `user`'s holder had proven the account's email before this
    /// sign-in: the confirmation link was followed, or the account was
    /// created or attached by a Firebase Google sign-in, which verifies the
    /// address it carries. An account like that is the athlete's own, so a
    /// first hosted Google sign-in attaches to it without retiring anything.
    async fn email_already_proven(&self, user: &User) -> AppResult<bool> {
        if user.firebase_uid.is_some() && user.auth_provider == GOOGLE_PROVIDER {
            return Ok(true);
        }
        self.data
            .repos()
            .email_verification
            .is_verified(user.id)
            .await
    }

    /// End every credential an account held before its email was proven.
    ///
    /// Anyone can register an address they do not hold and wait for its
    /// owner to sign in with Google. So when a sign-in proves an address the
    /// account never verified, the password is replaced by the
    /// federated-only marker, a Firebase link other than this sign-in's is
    /// dropped, and the device refresh tokens, `OAuth2` refresh tokens,
    /// connector grants, MCP tokens and API keys issued so far are revoked. The caller
    /// writes the user row. Access JWTs already minted run to their expiry,
    /// as after a password change.
    async fn retire_unproven_credentials(
        &self,
        user: &mut User,
        identity: &FederatedIdentity<'_>,
    ) -> AppResult<()> {
        let repos = self.data.repos();
        FEDERATED_ONLY_PASSWORD_HASH.clone_into(&mut user.password_hash);
        user.firebase_uid = identity.firebase_uid.map(str::to_owned);

        let sessions = repos
            .session_refresh_tokens
            .revoke_user_tokens(user.id, Utc::now())
            .await?;
        let user_id = user.id.to_string();

        let mut connector_tokens = 0_u64;
        let mut grants = 0_usize;
        for tenant in repos.tenants.list_for_user(user.id).await? {
            let tenant_id = tenant.id.to_string();
            connector_tokens += repos
                .oauth2_server
                .revoke_user_refresh_tokens(&user_id, &tenant_id)
                .await?;
            for grant in repos
                .oauth2_server
                .list_client_grants(&user_id, &tenant_id)
                .await?
            {
                if repos
                    .oauth2_server
                    .revoke_client_grant(&grant.id, &user_id, &tenant_id)
                    .await?
                {
                    grants += 1;
                }
            }
        }

        let mut mcp_tokens = 0_usize;
        for token in repos.user_mcp_tokens.list_tokens(user.id).await? {
            if !token.is_revoked {
                repos
                    .user_mcp_tokens
                    .revoke_token(&token.id, user.id)
                    .await?;
                mcp_tokens += 1;
            }
        }

        let mut api_keys = 0_usize;
        for key in repos.api_keys.get_for_user(user.id).await? {
            if key.is_active {
                repos.api_keys.deactivate(&key.id, user.id).await?;
                api_keys += 1;
            }
        }

        warn!(
            user_id = %user.id,
            sessions,
            connector_tokens,
            grants,
            mcp_tokens,
            api_keys,
            "Retired the credentials of an account whose email was never verified, now proven by a federated sign-in"
        );
        Ok(())
    }

    /// Record the identity's Google account id against `user_id`. A no-op
    /// when the sign-in carries none or it is already linked.
    async fn link_google_subject(
        &self,
        user_id: Uuid,
        identity: &FederatedIdentity<'_>,
    ) -> AppResult<()> {
        if let Some(subject) = identity.google_subject {
            self.data
                .repos()
                .federated_identities
                .link_subject(user_id, GOOGLE_PROVIDER, subject)
                .await?;
        }
        Ok(())
    }

    /// Create a new account for `identity`, under the registration approval
    /// posture, with its personal workspace.
    async fn create_federated_user(&self, identity: &FederatedIdentity<'_>) -> AppResult<User> {
        let email = identity.email.as_str();
        let (user_status, approved_at, approved_by) = self.determine_approval_status(email).await;
        let user_id = Uuid::new_v4();
        let display_name = identity
            .display_name
            .unwrap_or_else(|| email.split('@').next().unwrap_or("user"));

        // Step 1: Create user first - tenant membership managed via tenant_users table
        let now = Utc::now();
        let new_user = User {
            id: user_id,
            email: email.to_owned(),
            display_name: identity.display_name.map(str::to_owned),
            password_hash: FEDERATED_ONLY_PASSWORD_HASH.to_owned(),
            tier: UserTier::Starter,
            strava_token: None,
            created_at: now,
            last_active: now,
            is_active: true,
            user_status,
            is_admin: false,
            role: UserRole::User,
            approved_by,
            approved_at,
            firebase_uid: identity.firebase_uid.map(str::to_owned),
            auth_provider: identity.provider.to_owned(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: default_locale(),
            coaching_persona: CoachingPersona::default(),
            manages_roster: false,
            timezone: None,
            theme: None,
        };

        self.data.repos().users.create(&new_user).await?;

        // Step 2: Create personal tenant (adds user to tenant_users as owner)
        let tenant_id = self
            .create_personal_tenant(user_id, display_name, tiers::STARTER)
            .await?;

        info!(user_id = %user_id, provider = %identity.provider, "Federated user registered");

        // Social sign-ins raise the same signup event as the password
        // register endpoint, or they land silently and the acquisition count
        // under-reports them.
        info!(
            target: "notify",
            event = "user.signed_up",
            user_id = %user_id,
            tenant_id = %tenant_id,
            source = identity.signup_source.as_str(),
            "account created"
        );

        Ok(new_user)
    }

    /// Complete a federated login: generate the session JWT and update last active.
    async fn complete_federated_login(
        &self,
        user: &User,
        provider: &str,
    ) -> AppResult<LoginResponse> {
        // Ensure user has a tenant (auto-creates one for users without a tenant)
        let active_tenant_id = self.ensure_user_has_tenant(user).await?;
        let tenant_id_for_response = active_tenant_id.clone();

        let jwt_token = self
            .auth_manager
            .generate_token_with_tenant(user, &self.jwks_manager, active_tenant_id)
            .map_err(|e| AppError::internal(format!("Failed to generate token: {e}")))?;

        let expires_at = Utc::now() + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS);

        self.data.repos().users.update_last_active(user.id).await?;

        info!(user_id = %user.id, provider = %provider, "Federated login successful");

        let user_info = self.user_info(user, tenant_id_for_response).await;
        Ok(LoginResponse {
            jwt_token: Some(jwt_token),
            csrf_token: String::new(),
            expires_at: expires_at.to_rfc3339(),
            user: user_info,
        })
    }
}
