// ABOUTME: Authentication business logic for registration, login, and account management
// ABOUTME: Protocol-agnostic service reusable across REST, MCP, and A2A entry points
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use tokio::task;
use tracing::{debug, error, info, warn};

use crate::{
    admin::{FirebaseAuth, FirebaseClaims},
    constants::{error_messages, limits, tiers},
    context::{AuthContext, ConfigContext, DataContext},
    errors::{AppError, AppResult},
    models::{Tenant, TenantId, User, UserStatus, UserTier},
    permissions::UserRole,
    utils::errors::{auth_error, user_state_error, validation_error},
};

use crate::routes::auth::{
    FirebaseLoginRequest, LoginRequest, LoginResponse, RefreshTokenRequest, RegisterRequest,
    RegisterResponse, UserInfo,
};

// ---------------------------------------------------------------------------
// AuthService — domain logic for user authentication and registration
// ---------------------------------------------------------------------------

/// Authentication service encapsulating business logic for user lifecycle
///
/// Handles registration, credential login, Firebase SSO login, token refresh,
/// tenant provisioning, and approval-status determination. Accepts plain Rust
/// types (contexts) rather than HTTP framework extractors so it can be called
/// from REST, MCP, or A2A entry points.
#[derive(Clone)]
pub struct AuthService {
    auth: AuthContext,
    config: ConfigContext,
    data: DataContext,
}

impl AuthService {
    /// Creates a new authentication service
    #[must_use]
    pub const fn new(auth: AuthContext, config: ConfigContext, data: DataContext) -> Self {
        Self { auth, config, data }
    }

    /// Handle user registration
    ///
    /// Validates email/password, checks uniqueness, hashes credentials,
    /// creates the user with appropriate approval status, provisions a
    /// personal tenant, and sends an ops notification.
    ///
    /// # Errors
    /// Returns error if user validation fails or database operation fails
    #[tracing::instrument(skip(self, request), fields(route = "register"))]
    pub async fn register(&self, request: RegisterRequest) -> AppResult<RegisterResponse> {
        info!("User registration attempt");

        // Validate email format
        if !Self::is_valid_email(&request.email) {
            return Err(validation_error(error_messages::INVALID_EMAIL_FORMAT));
        }

        // Validate password strength
        if !Self::is_valid_password(&request.password) {
            return Err(validation_error(error_messages::PASSWORD_TOO_WEAK));
        }

        // Check if user already exists
        if let Ok(Some(_)) = self.data.repos().users.get_by_email(&request.email).await {
            return Err(user_state_error(error_messages::USER_ALREADY_EXISTS));
        }

        // Hash password
        let password_hash = bcrypt::hash(&request.password, bcrypt::DEFAULT_COST)
            .map_err(|e| AppError::internal(format!("Password hashing failed: {e}")))?;

        // Create user — determine_approval_status sets Pending or Active below
        let mut user = User::new(request.email.clone(), password_hash, request.display_name); // Safe: String ownership needed for user model

        // Check if user should be auto-approved (global setting or domain allow-list)
        let (status, approved_at) = self.determine_approval_status(&request.email).await;
        user.user_status = status;
        user.approved_at = approved_at;

        // Save user to database
        let user_id = self
            .data
            .repos()
            .users
            .create(&user)
            .await
            .map_err(|e| AppError::database(format!("Failed to create user: {e}")))?;

        // Create a personal tenant for the user (required for MCP operations)
        let display_name = user
            .display_name
            .as_deref()
            .unwrap_or_else(|| request.email.split('@').next().unwrap_or("user"));

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

        // Send ops notification for new user registration
        crate::ops_notifier().notify_user_registered(
            &user_id.to_string(),
            &request.email,
            &user.user_status.to_string(),
        );

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
    /// and sends an ops notification.
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
                debug!(email = %request.email, error = %e, "Login failed: user lookup error");
                AppError::auth_invalid("Invalid email or password")
            })?;

        // Verify password using spawn_blocking to avoid blocking async executor
        let password = request.password.clone();
        let password_hash = user.password_hash.clone();
        let is_valid = task::spawn_blocking(move || bcrypt::verify(&password, &password_hash))
            .await
            .map_err(|e| AppError::internal(format!("Password verification task failed: {e}")))?
            .map_err(|_| AppError::auth_invalid("Invalid email or password"))?;

        if !is_valid {
            warn!(
                email = %request.email,
                user_id = %user.id,
                "Failed login: invalid password"
            );
            crate::ops_notifier().notify_login_failed(&request.email);
            return Err(auth_error(error_messages::INVALID_CREDENTIALS));
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

        // Ensure user has a tenant (auto-creates one for admin setup/CLI users)
        let active_tenant_id = self.ensure_user_has_tenant(&user).await?;
        let tenant_id_for_response = active_tenant_id.clone();

        // Generate JWT token using RS256 with active_tenant_id
        let jwt_token = self
            .auth
            .auth_manager()
            .generate_token_with_tenant(&user, self.auth.jwks_manager(), active_tenant_id)
            .map_err(|e| AppError::auth_invalid(format!("Failed to generate token: {e}")))?;
        let expires_at =
            chrono::Utc::now() + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS); // Default 24h expiry

        info!(
            "User logged in successfully: {} ({})",
            request.email, user.id
        );

        // Send ops notification for user login
        crate::ops_notifier().notify_login(&request.email);

        Ok(LoginResponse {
            jwt_token: Some(jwt_token),
            csrf_token: String::new(), // Will be set by HTTP handler
            expires_at: expires_at.to_rfc3339(),
            user: UserInfo {
                id: user.id.to_string(),
                user_id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name,
                is_admin: user.is_admin,
                role: user.role.as_str().to_owned(),
                user_status: user.user_status.to_string(),
                tenant_id: tenant_id_for_response,
                created_at: user.created_at.to_rfc3339(),
                locale: user.locale.clone(),
            },
        })
    }

    /// Handle Firebase login - authenticate with Firebase ID token
    ///
    /// This method validates the Firebase ID token, finds or creates a user,
    /// and returns a JWT token for our authentication system.
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

        // Get the email from the claims (required)
        let email = claims
            .email
            .as_ref()
            .ok_or_else(|| AppError::auth_invalid("Firebase token missing email claim"))?;

        // Find or create user from Firebase claims
        let mut user = self.find_or_create_firebase_user(&claims, email).await?;

        // Block suspended users; pending users authenticate so the frontend
        // can show the "pending approval" page (user_status is in the response).
        Self::reject_if_suspended(&user)?;

        // Retroactively approve pending users whose domain now qualifies
        self.auto_approve_if_eligible(&mut user).await?;

        // Generate session and return response
        self.complete_firebase_login(&user, &claims.provider).await
    }

    /// Find existing user or create new one from Firebase claims
    async fn find_or_create_firebase_user(
        &self,
        claims: &FirebaseClaims,
        email: &str,
    ) -> AppResult<User> {
        // Try to find user by Firebase UID first
        if let Some(user) = self
            .data
            .repos()
            .users
            .get_by_firebase_uid(&claims.sub)
            .await?
        {
            tracing::info!(user_id = %user.id, firebase_uid = %claims.sub, "Found user by Firebase UID");
            return Ok(user);
        }

        // Check if user exists by email (might need linking)
        if let Some(mut user) = self.data.repos().users.get_by_email(email).await? {
            tracing::info!(user_id = %user.id, "Linking existing email user to Firebase UID");
            user.firebase_uid = Some(claims.sub.clone());
            user.auth_provider.clone_from(&claims.provider);
            self.data.repos().users.create(&user).await?;
            return Ok(user);
        }

        // Create new user from Firebase claims
        self.create_firebase_user(claims, email).await
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
        let tenant_id = TenantId::new();
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
        Ok(tenant_id)
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
        let config = self.config.config();

        // Environment variable takes precedence when explicitly set
        if config.app_behavior.auto_approve_users_from_env {
            return config.app_behavior.auto_approve_users;
        }

        // Fall back to database setting if present
        match self.data.database().is_auto_approval_enabled().await {
            Ok(Some(db_setting)) => db_setting,
            Ok(None) => config.app_behavior.auto_approve_users,
            Err(e) => {
                tracing::warn!(
                    "Failed to check auto-approval setting, falling back to config: {e}"
                );
                config.app_behavior.auto_approve_users
            }
        }
    }

    /// Check if a specific email should be auto-approved.
    ///
    /// Returns true when:
    /// - Global auto-approval is enabled (`AUTO_APPROVE_USERS=true`), OR
    /// - The email domain is in the `AUTO_APPROVE_DOMAINS` allow-list
    async fn should_auto_approve_email(&self, email: &str) -> bool {
        if self.is_auto_approval_enabled().await {
            return true;
        }

        let domains = &self.config.config().app_behavior.auto_approve_domains;
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

    /// Determine user approval status based on auto-approval setting and email domain
    async fn determine_approval_status(
        &self,
        email: &str,
    ) -> (UserStatus, Option<chrono::DateTime<Utc>>) {
        let now = Utc::now();
        if self.should_auto_approve_email(email).await {
            tracing::debug!("Auto-approval granted for new user");
            (UserStatus::Active, Some(now))
        } else {
            (UserStatus::Pending, None)
        }
    }

    /// Create a new user from Firebase claims
    async fn create_firebase_user(&self, claims: &FirebaseClaims, email: &str) -> AppResult<User> {
        tracing::info!(firebase_uid = %claims.sub, "Creating new Firebase user");

        let (user_status, approved_at) = self.determine_approval_status(email).await;
        let user_id = uuid::Uuid::new_v4();
        let display_name = claims
            .name
            .as_deref()
            .unwrap_or_else(|| email.split('@').next().unwrap_or("user"));

        // Step 1: Create user first - tenant membership managed via tenant_users table
        let now = Utc::now();
        let new_user = User {
            id: user_id,
            email: email.to_owned(),
            display_name: claims.name.clone(),
            password_hash: "!firebase-auth-only!".to_owned(),
            tier: UserTier::Starter,
            strava_token: None,
            fitbit_token: None,
            created_at: now,
            last_active: now,
            is_active: true,
            user_status,
            is_admin: false,
            role: UserRole::User,
            approved_by: None,
            approved_at,
            firebase_uid: Some(claims.sub.clone()),
            auth_provider: claims.provider.clone(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: "fr".to_owned(),
            default_coach_id: None,
        };

        self.data.repos().users.create(&new_user).await?;

        // Step 2: Create personal tenant (adds user to tenant_users as owner)
        self.create_personal_tenant(user_id, display_name, tiers::STARTER)
            .await?;

        info!(firebase_uid = %claims.sub, user_id = %user_id, "Firebase user registered");
        Ok(new_user)
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

    /// Re-evaluate domain auto-approval for existing Pending users.
    ///
    /// When `AUTO_APPROVE_DOMAINS` is updated after a user registered,
    /// their account stays Pending forever. This method promotes them to
    /// Active on their next login if their email domain now qualifies.
    async fn auto_approve_if_eligible(&self, user: &mut User) -> AppResult<()> {
        if user.user_status != UserStatus::Pending {
            return Ok(());
        }

        if !self.should_auto_approve_email(&user.email).await {
            return Ok(());
        }

        tracing::info!(
            user_id = %user.id,
            email = %user.email,
            "Retroactive domain auto-approval for pending user"
        );

        let updated = self
            .data
            .repos()
            .users
            .update_status(user.id, UserStatus::Active, None)
            .await?;
        user.user_status = updated.user_status;
        user.approved_at = updated.approved_at;

        Ok(())
    }

    /// Complete Firebase login: generate JWT and update last active
    async fn complete_firebase_login(
        &self,
        user: &User,
        provider: &str,
    ) -> AppResult<LoginResponse> {
        // Ensure user has a tenant (auto-creates one for users without a tenant)
        let active_tenant_id = self.ensure_user_has_tenant(user).await?;
        let tenant_id_for_response = active_tenant_id.clone();

        let jwt_token = self
            .auth
            .auth_manager()
            .generate_token_with_tenant(user, self.auth.jwks_manager(), active_tenant_id)
            .map_err(|e| AppError::auth_invalid(format!("Failed to generate token: {e}")))?;

        let expires_at = Utc::now() + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS);

        self.data.repos().users.update_last_active(user.id).await?;

        tracing::info!(user_id = %user.id, provider = %provider, "Firebase login successful");

        Ok(LoginResponse {
            jwt_token: Some(jwt_token),
            csrf_token: String::new(),
            expires_at: expires_at.to_rfc3339(),
            user: UserInfo {
                id: user.id.to_string(),
                user_id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                is_admin: user.is_admin,
                role: user.role.as_str().to_owned(),
                user_status: user.user_status.to_string(),
                tenant_id: tenant_id_for_response,
                created_at: user.created_at.to_rfc3339(),
                locale: user.locale.clone(),
            },
        })
    }

    /// Handle token refresh
    ///
    /// Validates the existing JWT, verifies user identity, checks account
    /// status, and issues a fresh token with updated expiry.
    ///
    /// # Errors
    /// Returns error if refresh token is invalid or token generation fails
    pub async fn refresh_token(&self, request: RefreshTokenRequest) -> AppResult<LoginResponse> {
        info!("Token refresh attempt for user with refresh token");

        // Extract user from refresh token using RS256 validation
        let token_claims = self
            .auth
            .auth_manager()
            .validate_token(&request.token, self.auth.jwks_manager())
            .map_err(|_| AppError::auth_invalid("Invalid or expired token"))?;
        let user_id = uuid::Uuid::parse_str(&token_claims.sub)
            .map_err(|e| AppError::auth_invalid(format!("Invalid token format: {e}")))?;

        // Validate that the user_id matches the one in the request
        let request_user_id = uuid::Uuid::parse_str(&request.user_id)?;
        if user_id != request_user_id {
            return Err(AppError::auth_invalid("User ID mismatch"));
        }

        // Get user from database
        let user = self
            .data
            .repos()
            .users
            .get_global(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| AppError::not_found("User"))?;

        // Block suspended users from refreshing tokens.
        // Pending users can refresh so the frontend can poll for status changes.
        Self::reject_if_suspended(&user)?;

        // Ensure user has a tenant (auto-creates one for admin setup/CLI users)
        let active_tenant_id = self.ensure_user_has_tenant(&user).await?;
        let tenant_id_for_response = active_tenant_id.clone();

        // Generate new JWT token using RS256 with active_tenant_id
        let new_jwt_token = self
            .auth
            .auth_manager()
            .generate_token_with_tenant(&user, self.auth.jwks_manager(), active_tenant_id)
            .map_err(|e| AppError::auth_invalid(format!("Failed to generate token: {e}")))?;
        let expires_at =
            chrono::Utc::now() + chrono::Duration::hours(limits::DEFAULT_SESSION_HOURS);

        // Update last active timestamp
        self.data
            .repos()
            .users
            .update_last_active(user.id)
            .await
            .map_err(|e| AppError::database(format!("Failed to update last active: {e}")))?;

        info!("Token refreshed successfully for user: {}", user.id);

        Ok(LoginResponse {
            jwt_token: Some(new_jwt_token),
            csrf_token: String::new(), // Will be set by HTTP handler
            expires_at: expires_at.to_rfc3339(),
            user: UserInfo {
                id: user.id.to_string(),
                user_id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name,
                is_admin: user.is_admin,
                role: user.role.as_str().to_owned(),
                user_status: user.user_status.to_string(),
                tenant_id: tenant_id_for_response,
                created_at: user.created_at.to_rfc3339(),
                locale: user.locale.clone(),
            },
        })
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

    /// Verify a plaintext password against a bcrypt hash.
    ///
    /// Runs the CPU-intensive bcrypt comparison on a blocking thread
    /// to avoid stalling the async executor.
    ///
    /// # Errors
    /// Returns error if the password does not match or the bcrypt operation fails
    pub async fn verify_password(password: String, hash: String) -> AppResult<bool> {
        let is_valid = task::spawn_blocking(move || bcrypt::verify(&password, &hash))
            .await
            .map_err(|e| AppError::internal(format!("Password verification task failed: {e}")))?
            .map_err(|_| AppError::auth_invalid("Invalid email or password"))?;
        Ok(is_valid)
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
