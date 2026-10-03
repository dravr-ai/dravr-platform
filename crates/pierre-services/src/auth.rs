// ABOUTME: Authentication business logic for registration, login, and account management
// ABOUTME: Protocol-agnostic service reusable across REST, MCP, and A2A entry points
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use chrono::Utc;
use tokio::task;
use tracing::{debug, error, info, warn};

use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_auth::dto::auth::{
    FirebaseLoginRequest, LoginRequest, LoginResponse, RegisterRequest, RegisterResponse, UserInfo,
};
use pierre_auth::firebase::FirebaseAuth;
use pierre_auth::password::verify_password;
use pierre_auth::refresh_rotation::{
    consume_or_revoke_family, generate_refresh_token, refresh_token_lifetime,
};
use pierre_config::environment::ServerConfig;
use pierre_core::constants::{error_messages, limits, tiers};
use pierre_core::error_helpers::{user_state_error, validation_error};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    normalize_email, PreApprovedEmail, SessionRefreshToken, Tenant, TenantId, User, UserStatus,
};
use pierre_runtime_context::DataContext;

use crate::default_agent::first_athlete_facing;

/// Sign-in through an external identity (Firebase, Google) and the account rules both follow
mod federated;

pub use federated::{FederatedIdentity, SignupSource};

// ---------------------------------------------------------------------------
// AuthService — domain logic for user authentication and registration
// ---------------------------------------------------------------------------

/// The refusal a login answers when the credentials it was given do not open
/// the account. It names neither the account nor why: a client reads only the
/// generic "credentials are invalid" an [`ErrorCode::AuthInvalid`] carries.
///
/// [`ErrorCode::AuthInvalid`]: pierre_core::errors::ErrorCode::AuthInvalid
fn invalid_credentials() -> AppError {
    AppError::auth_invalid(format!(
        "Authentication failed: {}",
        error_messages::INVALID_CREDENTIALS
    ))
}

/// What a refresh-token exchange yields: the same response a login gives,
/// plus the successor token the device must hold from now on.
#[derive(Debug)]
pub struct RefreshedSession {
    /// Fresh JWT, expiry and user, exactly as a login answers.
    pub login: LoginResponse,
    /// The successor refresh token. The one that was presented is dead.
    pub refresh_token: String,
}

/// Authentication service encapsulating business logic for user lifecycle
///
/// Handles registration, credential login, Firebase SSO login, refresh-token
/// issue and exchange, tenant provisioning, and approval-status
/// determination. Accepts narrow Rust types rather than HTTP framework
/// extractors so it can be called from REST, MCP, or A2A entry points.
#[derive(Clone)]
pub struct AuthService {
    auth_manager: Arc<AuthManager>,
    jwks_manager: Arc<JwksManager>,
    config: Arc<ServerConfig>,
    data: DataContext,
}

impl AuthService {
    /// Creates a new authentication service
    #[must_use]
    pub const fn new(
        auth_manager: Arc<AuthManager>,
        jwks_manager: Arc<JwksManager>,
        config: Arc<ServerConfig>,
        data: DataContext,
    ) -> Self {
        Self {
            auth_manager,
            jwks_manager,
            config,
            data,
        }
    }

    /// Handle user registration
    ///
    /// Normalizes the email ([`normalize_email`]), validates email/password,
    /// checks uniqueness in any casing, hashes credentials, creates the user
    /// with appropriate approval status, provisions a personal tenant, and
    /// raises the `user.signed_up` notify event.
    ///
    /// # Errors
    /// Returns error if user validation fails or database operation fails
    #[tracing::instrument(skip(self, request), fields(route = "register"))]
    pub async fn register(&self, request: RegisterRequest) -> AppResult<RegisterResponse> {
        info!("User registration attempt");

        // One form for the address from here on: the uniqueness check, the
        // stored row, the pre-approval match and the domain allow-list all
        // read it, so `Jane@X.com` and `jane@x.com` are one person to each.
        let email = normalize_email(&request.email);

        // Validate email format
        if !Self::is_valid_email(&email) {
            return Err(validation_error(error_messages::INVALID_EMAIL_FORMAT));
        }

        // Validate password strength
        if !Self::is_valid_password(&request.password) {
            return Err(validation_error(error_messages::PASSWORD_TOO_WEAK));
        }

        // Check if user already exists, in any casing
        if let Ok(Some(_)) = self.data.repos().users.get_by_email(&email).await {
            return Err(user_state_error(error_messages::USER_ALREADY_EXISTS));
        }

        // Hash password
        let password_hash = bcrypt::hash(&request.password, bcrypt::DEFAULT_COST)
            .map_err(|e| AppError::internal(format!("Password hashing failed: {e}")))?;

        // Create user — determine_approval_status sets Pending or Active below
        let mut user = User::new(email.clone(), password_hash, request.display_name);

        // Check if user should be auto-approved (global setting, domain
        // allow-list, or a per-email pre-approval carrying its operator)
        let (status, approved_at, approved_by) = self.determine_approval_status(&email).await;
        user.user_status = status;
        user.approved_at = approved_at;
        user.approved_by = approved_by;

        // Propagated, not re-wrapped: `create` is insert-only now and reports a
        // duplicate email as `invalid_input`. Wrapping it in `AppError::database`
        // turned that into a 500, so the one case a caller can actually act on —
        // "that address is taken" — arrived as a server fault. The bare `?` is
        // what the Firebase-link branch below already does.
        let user_id = self.data.repos().users.create(&user).await?;

        // Create a personal tenant for the user (required for MCP operations)
        let display_name = user
            .display_name
            .as_deref()
            .unwrap_or_else(|| email.split('@').next().unwrap_or("user"));

        let tenant_id = self
            .create_personal_tenant(user_id, display_name, tiers::STARTER)
            .await?;

        // Assign user to their personal tenant
        self.data
            .repos()
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .map_err(|e| {
                error!("Failed to assign user to tenant: {}", e);
                AppError::database(format!("Failed to assign tenant: {e}"))
            })?;

        info!(user_id = %user_id, "User registered successfully");

        let message = if user.user_status == UserStatus::Active {
            "User registered successfully. Your account is ready to use.".to_owned()
        } else {
            "User registered successfully. Your account is pending admin approval.".to_owned()
        };

        Ok(RegisterResponse {
            user_id: user_id.to_string(),
            user_status: user.user_status,
            display_name: user.display_name,
            message,
        })
    }

    /// Handle user login with email/password credentials
    ///
    /// Looks up the user, verifies the password (off the async executor),
    /// checks account status, auto-approves if eligible, generates a JWT,
    /// and raises the `user.login` notify event.
    ///
    /// # Errors
    /// Returns error if authentication fails or token generation fails
    #[tracing::instrument(skip(self, request), fields(route = "login"))]
    pub async fn login(&self, request: LoginRequest) -> AppResult<LoginResponse> {
        debug!("User login attempt");

        // Get user from database
        let user = self
            .data
            .repos()
            .users
            .get_by_email_required(&request.email)
            .await
            .map_err(|e| {
                // An unknown email is the caller's; anything else is the lookup
                // failing, which is an outage and not a wrong password.
                if e.is_server_fault() {
                    error!(error = %e, "Login could not complete: user lookup failed");
                    return e;
                }
                debug!(email = %request.email, error = %e, "Login refused: user lookup");
                AppError::auth_invalid("Invalid email or password")
            })?;

        let is_valid =
            verify_password(request.password.clone(), user.password_hash.clone()).await?;

        if !is_valid {
            warn!(user_id = %user.id, "Failed login: invalid password");
            info!(
                target: "notify",
                event = "user.login_failed",
                reason = "invalid_password",
                "login rejected"
            );
            return Err(invalid_credentials());
        }

        // Block suspended users; pending users authenticate so the frontend
        // can show the "pending approval" page (user_status is in the response).
        // The auth middleware enforces status on subsequent API calls.
        Self::reject_if_suspended(&user)?;

        // Retroactively approve pending users whose domain now qualifies
        let mut user = user;
        self.auto_approve_if_eligible(&mut user).await?;

        // Update last active timestamp
        self.data
            .repos()
            .users
            .update_last_active(user.id)
            .await
            .map_err(|e| AppError::database(format!("Failed to update last active: {e}")))?;

        // Persist the user's IANA timezone if the client reported one
        // that differs from the stored value. The web/mobile clients
        // capture this via Intl.DateTimeFormat().resolvedOptions().
        // timeZone on every login attempt; the chat prompt-assembly
        // stage reads it back to resolve `{{CURRENT_DATE}}` to the
        // user's local calendar day. Same-value writes are skipped to
        // avoid pointless DB churn on every login.
        if let Some(ref tz) = request.timezone {
            if !tz.is_empty() && user.timezone.as_deref() != Some(tz.as_str()) {
                if let Err(e) = self.data.repos().users.set_timezone(user.id, tz).await {
                    warn!(user_id = %user.id, error = %e, "Failed to persist user timezone");
                }
            }
        }

        // Ensure user has a tenant (auto-creates one for admin setup/CLI users)
        let active_tenant_id = self.ensure_user_has_tenant(&user).await?;
        let tenant_id_for_response = active_tenant_id.clone();

        // Generate JWT token using RS256 with active_tenant_id
        let jwt_token = self
            .auth_manager
            .generate_token_with_tenant(&user, &self.jwks_manager, active_tenant_id)
            .map_err(|e| AppError::internal(format!("Failed to generate token: {e}")))?;
        let expires_at =
            chrono::Utc::now() + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS); // Default 24h expiry

        info!(user_id = %user.id, "User logged in successfully");

        let user_info = self.user_info(&user, tenant_id_for_response).await;
        Ok(LoginResponse {
            jwt_token: Some(jwt_token),
            csrf_token: String::new(), // Will be set by HTTP handler
            expires_at: expires_at.to_rfc3339(),
            user: user_info,
        })
    }

    /// Handle Firebase login - authenticate with Firebase ID token
    ///
    /// Validates the Firebase ID token, then signs the person it names in
    /// through [`Self::login_with_federated_identity`]: the account its
    /// Firebase UID or Google account id is linked to, else the account its
    /// proven email names, else a new account.
    ///
    /// # Errors
    /// Returns error if Firebase validation fails, or user creation fails
    pub async fn login_with_firebase(
        &self,
        request: FirebaseLoginRequest,
        firebase_auth: &FirebaseAuth,
    ) -> AppResult<LoginResponse> {
        tracing::info!("Firebase login attempt");

        // Validate the Firebase ID token
        let claims = firebase_auth.validate_token(&request.id_token).await?;

        // Get the email from the claims (required), in its stored form: the
        // account it names is found, or created, whatever casing the provider
        // reports it in.
        let email = claims
            .email
            .as_deref()
            .map(normalize_email)
            .ok_or_else(|| AppError::auth_invalid("Firebase token missing email claim"))?;

        self.login_with_federated_identity(FederatedIdentity {
            firebase_uid: Some(&claims.sub),
            google_subject: claims.google_subject(),
            email,
            email_verified: claims.email_verified == Some(true),
            display_name: claims.name.as_deref(),
            provider: &claims.provider,
            signup_source: SignupSource::Firebase,
        })
        .await
    }

    /// Create a personal tenant for a user (required for MCP operations)
    ///
    /// # Errors
    /// Returns error if tenant creation fails
    async fn create_personal_tenant(
        &self,
        user_id: uuid::Uuid,
        display_name: &str,
        plan: &str,
    ) -> AppResult<TenantId> {
        let tenant_id = TenantId::generate();
        let tenant_name = format!("{display_name}'s Workspace");
        let tenant_slug = format!("user-{}", user_id.as_simple());
        let now = Utc::now();

        let tenant = Tenant {
            id: tenant_id,
            name: tenant_name.clone(),
            slug: tenant_slug,
            domain: None,
            plan: plan.to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        };

        self.data
            .repos()
            .tenants
            .create(&tenant)
            .await
            .map_err(|e| {
                error!(
                    "Failed to create personal tenant for user {}: {}",
                    user_id, e
                );
                AppError::database(format!("Failed to create personal tenant: {e}"))
            })?;

        debug!("Created personal tenant: {} ({})", tenant_name, tenant_id);

        self.select_starter_agent(tenant_id, user_id).await;

        Ok(tenant_id)
    }

    /// Give a brand-new workspace an agent to talk to.
    ///
    /// Without this a user reaches chat with nothing selected, which reads to
    /// every downstream check as "not onboarded" — the state that had the
    /// messaging surface re-running the agent proposal at someone who had
    /// already finished the web wizard. The agent proposal still runs and still
    /// lets them choose; this only ensures the floor is a working conversation
    /// rather than an empty one.
    ///
    /// Picks the first system agent deterministically. Not a recommendation —
    /// the proposal does that once there is data to reason about — just a
    /// sensible default that any later selection replaces.
    ///
    /// Best-effort: registration has already succeeded, and failing it over a
    /// default would trade a working account for a cosmetic one.
    ///
    /// The account has not said yet whether it trains, so the starter is an
    /// athlete-facing agent; a coach who does not train has it released when
    /// they answer (see [`crate::intake::release_coach_only_agent`]), and
    /// their thread resolves the roster agent from then on.
    async fn select_starter_agent(&self, tenant_id: TenantId, user_id: uuid::Uuid) {
        let agents = match self.data.repos().agents.list_system_agents(tenant_id).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "could not list system agents for the starter selection");
                return;
            }
        };

        let Some(first) = first_athlete_facing(&agents) else {
            // A deployment with no system agents seeded yet. The proposal will
            // still offer whatever exists by the time the user gets there.
            return;
        };

        let agent_id = first.id.to_string();
        if let Err(e) = self
            .data
            .repos()
            .tenants
            .set_selected_agent(tenant_id, user_id, Some(&agent_id))
            .await
        {
            warn!(error = %e, "could not set the starter coach selection");
        }
    }

    /// Ensure user has at least one tenant, creating a personal tenant if needed.
    ///
    /// Returns the `tenant_id` to use as `active_tenant_id` in JWT claims.
    /// Users created via admin setup or CLI may not have a tenant; this method
    /// auto-creates one on first login so route handlers can rely on `active_tenant_id`.
    ///
    /// # Errors
    /// Returns error if database operations fail
    async fn ensure_user_has_tenant(&self, user: &User) -> AppResult<Option<String>> {
        let tenants = self
            .data
            .repos()
            .tenants
            .list_for_user(user.id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

        if let Some(first) = tenants.first() {
            return Ok(Some(first.id.to_string()));
        }

        // User has no tenant — create a personal one (handles admin setup, CLI users)
        let display_name = user
            .display_name
            .as_deref()
            .unwrap_or_else(|| user.email.split('@').next().unwrap_or("user"));

        let tenant_id = self
            .create_personal_tenant(user.id, display_name, tiers::STARTER)
            .await?;

        self.data
            .repos()
            .users
            .update_tenant_id(user.id, tenant_id)
            .await
            .map_err(|e| {
                error!("Failed to assign user to tenant: {}", e);
                AppError::database(format!("Failed to assign tenant: {e}"))
            })?;

        info!(user_id = %user.id, tenant_id = %tenant_id, "Auto-created personal tenant on login");

        Ok(Some(tenant_id.to_string()))
    }

    /// Check if global auto-approval is enabled (ignores domain allow-list).
    ///
    /// Precedence order:
    /// 1. Environment variable (if explicitly set via `AUTO_APPROVE_USERS`)
    /// 2. Database setting (if present in `system_settings` table)
    /// 3. Default value (false)
    async fn is_auto_approval_enabled(&self) -> bool {
        // Environment variable takes precedence when explicitly set
        if self.config.app_behavior.auto_approve_users_from_env {
            return self.config.app_behavior.auto_approve_users;
        }

        // Fall back to database setting if present
        match self.data.database().is_auto_approval_enabled().await {
            Ok(Some(db_setting)) => db_setting,
            Ok(None) => self.config.app_behavior.auto_approve_users,
            Err(e) => {
                tracing::warn!(
                    "Failed to check auto-approval setting, falling back to config: {e}"
                );
                self.config.app_behavior.auto_approve_users
            }
        }
    }

    /// Fetch the standing per-email pre-approval for `email`, if any.
    ///
    /// Lookup failures degrade to `None` with a warning: a broken allow-list
    /// read must leave registration in the safe posture (pending queue),
    /// never fail it.
    async fn pre_approval_for(&self, email: &str) -> Option<PreApprovedEmail> {
        match self.data.repos().pre_approved_emails.get(email).await {
            Ok(entry) => entry,
            Err(e) => {
                warn!("Pre-approved email lookup failed, treating as not allowed: {e}");
                None
            }
        }
    }

    /// Check if a specific email should be auto-approved.
    ///
    /// Returns true when:
    /// - Global auto-approval is enabled (`AUTO_APPROVE_USERS=true`), OR
    /// - The address is on the per-email pre-approval list (`pierre-cli user
    ///   allow --email X`), OR
    /// - The email domain is in the `AUTO_APPROVE_DOMAINS` allow-list
    async fn should_auto_approve_email(&self, email: &str) -> bool {
        if self.is_auto_approval_enabled().await {
            return true;
        }

        if self.pre_approval_for(email).await.is_some() {
            tracing::debug!("Auto-approving pre-approved email");
            return true;
        }

        let domains = &self.config.app_behavior.auto_approve_domains;
        if domains.is_empty() {
            return false;
        }

        // Extract domain from email (lowercase for case-insensitive comparison)
        let email_domain = match email.rsplit_once('@') {
            Some((_, domain)) => domain.to_lowercase(),
            None => return false,
        };

        let approved = domains.iter().any(|d| d == &email_domain);
        if approved {
            tracing::debug!(email_domain = %email_domain, "Auto-approving user from allowed domain");
        }
        approved
    }

    /// Determine user approval status from the per-email pre-approval list,
    /// the global auto-approval setting, and the email-domain allow-list.
    ///
    /// A per-email pre-approval also carries the operator who recorded it, so
    /// the new account's `approved_by` attributes the decision to a person;
    /// the global switch and domain list are policy, not a person, and leave
    /// it `None`.
    async fn determine_approval_status(
        &self,
        email: &str,
    ) -> (
        UserStatus,
        Option<chrono::DateTime<Utc>>,
        Option<uuid::Uuid>,
    ) {
        let now = Utc::now();
        if let Some(entry) = self.pre_approval_for(email).await {
            tracing::debug!("Auto-approval granted via pre-approved email");
            return (UserStatus::Active, Some(now), entry.allowed_by);
        }
        if self.should_auto_approve_email(email).await {
            tracing::debug!("Auto-approval granted for new user");
            (UserStatus::Active, Some(now), None)
        } else {
            (UserStatus::Pending, None, None)
        }
    }

    /// Reject login for suspended users.
    ///
    /// Pending users are allowed to authenticate so the frontend can show
    /// the "pending approval" page. The auth middleware gates API access.
    fn reject_if_suspended(user: &User) -> AppResult<()> {
        if user.user_status == UserStatus::Suspended {
            tracing::warn!(user_id = %user.id, "Login denied: account suspended");
            return Err(AppError::account_suspended(
                "Your account has been suspended",
            ));
        }
        Ok(())
    }

    /// Re-evaluate auto-approval for existing Pending users.
    ///
    /// When `AUTO_APPROVE_DOMAINS` gains a domain — or an operator allow-lists
    /// the exact address — after a user registered, their account stays
    /// Pending forever. This method promotes them to Active on their next
    /// login if their email now qualifies, attributing `approved_by` to the
    /// allow-listing operator when the promotion comes from a per-email
    /// pre-approval.
    async fn auto_approve_if_eligible(&self, user: &mut User) -> AppResult<()> {
        if user.user_status != UserStatus::Pending {
            return Ok(());
        }

        if !self.should_auto_approve_email(&user.email).await {
            return Ok(());
        }

        tracing::info!(user_id = %user.id, "Retroactive auto-approval for pending user");

        let approver = self
            .pre_approval_for(&user.email)
            .await
            .and_then(|entry| entry.allowed_by);
        let updated = self
            .data
            .repos()
            .users
            .update_status(user.id, UserStatus::Active, approver)
            .await?;
        user.user_status = updated.user_status;
        user.approved_at = updated.approved_at;

        Ok(())
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
    /// Flip `AUTO_APPROVE_USERS` on and the same call promotes them immediately,
    /// with no second implementation for "open signup".
    ///
    /// Returns the status the user now holds.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cannot be loaded or the status write fails.
    pub async fn apply_approval_after_verification(
        &self,
        user_id: uuid::Uuid,
    ) -> AppResult<UserStatus> {
        let Some(mut user) = self.data.repos().users.get_global(user_id).await? else {
            return Err(AppError::not_found("User not found"));
        };

        // Suspended accounts are never revived by confirming an email.
        if user.user_status != UserStatus::Pending {
            return Ok(user.user_status);
        }

        self.auto_approve_if_eligible(&mut user).await?;
        Ok(user.user_status)
    }

    /// Open a refresh-token family for a device that just logged in.
    ///
    /// The JWT the login minted lasts `JWT_EXPIRY_HOURS`; this is what the
    /// device holds past that. It is returned once, in plaintext, and stored
    /// only as its HMAC. `tenant_id` is the tenant the login resolved, so the
    /// JWTs a refresh mints later carry the same one.
    ///
    /// # Errors
    /// Returns an error if the RNG fails or the token cannot be stored.
    pub async fn issue_refresh_token(
        &self,
        user_id: uuid::Uuid,
        tenant_id: Option<String>,
    ) -> AppResult<String> {
        let now = Utc::now();
        let record = SessionRefreshToken {
            family_id: uuid::Uuid::new_v4().to_string(),
            user_id,
            tenant_id,
            created_at: now,
            expires_at: now + self.refresh_token_lifetime(),
        };
        self.store_refresh_token(&record).await
    }

    /// Exchange a refresh token for a fresh JWT and its own successor.
    ///
    /// The presented token is consumed in the same statement that reads it,
    /// so it works exactly once; the successor joins the same family. A token
    /// that no longer exchanges but is still on record was rotated out and
    /// then presented again — the shape a stolen credential takes once the
    /// legitimate device has moved on — and revokes its whole family, live
    /// successor included. Every failure answers the same `AuthInvalid` so a
    /// caller cannot tell an expired token from a foreign one.
    ///
    /// # Errors
    /// Returns `AuthInvalid` for an unknown, expired, revoked or replayed
    /// token, `AccountSuspended` for a suspended user, and a database error
    /// when the exchange cannot be recorded.
    pub async fn refresh_session(&self, presented: &str) -> AppResult<RefreshedSession> {
        let now = Utc::now();
        let repos = self.data.repos();
        let Some(record) = consume_or_revoke_family(
            repos.session_refresh_tokens.consume_token(presented, now),
            || {
                repos
                    .session_refresh_tokens
                    .revoke_token_family(presented, now)
            },
        )
        .await?
        else {
            return Err(AppError::auth_invalid("Invalid or expired refresh token"));
        };

        let user = repos
            .users
            .get_global(record.user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| AppError::not_found("User"))?;

        // A suspended account cannot extend its session; a pending one can,
        // so the app keeps polling for the approval.
        Self::reject_if_suspended(&user)?;

        // The tenant the login resolved, while the user still belongs to it.
        // A member removed since then, or a session opened before the user had
        // a tenant, resolves one now the way a login would — the family keeps
        // sliding, so a tenant it recorded is never trusted past its membership.
        let tenant_id = match record.tenant_id {
            Some(tenant_id) if self.is_tenant_member(user.id, &tenant_id).await? => Some(tenant_id),
            _ => self.ensure_user_has_tenant(&user).await?,
        };

        let jwt_token = self
            .auth_manager
            .generate_token_with_tenant(&user, &self.jwks_manager, tenant_id.clone())
            .map_err(|e| AppError::internal(format!("Failed to generate token: {e}")))?;
        let expires_at = now + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS);

        let successor = SessionRefreshToken {
            family_id: record.family_id,
            user_id: user.id,
            tenant_id: tenant_id.clone(),
            created_at: now,
            expires_at: now + self.refresh_token_lifetime(),
        };
        let refresh_token = self.store_refresh_token(&successor).await?;

        repos
            .users
            .update_last_active(user.id)
            .await
            .map_err(|e| AppError::database(format!("Failed to update last active: {e}")))?;

        info!(user_id = %user.id, "Session refreshed");

        let user_info = self.user_info(&user, tenant_id).await;
        Ok(RefreshedSession {
            login: LoginResponse {
                jwt_token: Some(jwt_token),
                csrf_token: String::new(), // Will be set by HTTP handler
                expires_at: expires_at.to_rfc3339(),
                user: user_info,
            },
            refresh_token,
        })
    }

    /// How long a freshly issued refresh token stays exchangeable.
    fn refresh_token_lifetime(&self) -> chrono::Duration {
        refresh_token_lifetime(self.config.auth.refresh_token_expiry_days)
    }

    /// Whether `user_id` still holds a membership row in the tenant a JWT or
    /// refresh token recorded. A value that is not a tenant id is no
    /// membership.
    ///
    /// # Errors
    /// Returns a database error when the lookup fails.
    async fn is_tenant_member(&self, user_id: uuid::Uuid, tenant_id: &str) -> AppResult<bool> {
        let Ok(tenant_id) = TenantId::parse_str(tenant_id) else {
            return Ok(false);
        };
        Ok(self
            .data
            .repos()
            .tenants
            .get_user_role(user_id, tenant_id)
            .await?
            .is_some())
    }

    /// Mint a token, store its record, and hand the plaintext back — the same
    /// value shape the `OAuth2` server's refresh tokens take.
    async fn store_refresh_token(&self, record: &SessionRefreshToken) -> AppResult<String> {
        let token = generate_refresh_token()?;
        self.data
            .repos()
            .session_refresh_tokens
            .store_token(&token, record)
            .await?;
        Ok(token)
    }

    /// The user as a login or refresh response describes them.
    async fn user_info(&self, user: &User, tenant_id: Option<String>) -> UserInfo {
        UserInfo {
            id: user.id.to_string(),
            user_id: user.id.to_string(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            is_admin: user.is_admin,
            role: user.role.as_str().to_owned(),
            user_status: user.user_status.to_string(),
            tenant_id,
            // `.ok()` maps a failed lookup to None — "we did not resolve it" —
            // rather than false, which would claim the address is unconfirmed.
            email_verified: self
                .data
                .repos()
                .email_verification
                .is_verified(user.id)
                .await
                .ok(),
            created_at: user.created_at.to_rfc3339(),
            locale: user.locale.clone(),
            coaching_persona: user.coaching_persona.as_str().to_owned(),
            manages_roster: user.manages_roster,
        }
    }

    /// Validate email format
    #[must_use]
    pub fn is_valid_email(email: &str) -> bool {
        // Simple email validation
        if email.len() <= 5 {
            return false;
        }
        let Some(at_pos) = email.find('@') else {
            return false;
        };
        if at_pos == 0 || at_pos == email.len() - 1 {
            return false; // @ at start or end
        }
        let domain_part = &email[at_pos + 1..];
        domain_part.contains('.')
    }

    /// Validate password strength
    #[must_use]
    pub const fn is_valid_password(password: &str) -> bool {
        password.len() >= 8
    }

    /// Hash a plaintext password with bcrypt.
    ///
    /// Runs on a blocking thread to avoid stalling the async executor.
    ///
    /// # Errors
    /// Returns error if the hashing operation fails
    pub async fn hash_password(password: String) -> AppResult<String> {
        task::spawn_blocking(move || bcrypt::hash(&password, bcrypt::DEFAULT_COST))
            .await
            .map_err(|e| AppError::internal(format!("Password hashing task failed: {e}")))?
            .map_err(|e| AppError::internal(format!("Password hashing failed: {e}")))
    }
}
