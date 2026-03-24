// ABOUTME: Authentication service and HTTP handlers for registration, login, and account management
// ABOUTME: Contains AuthService business logic and Axum handler functions for auth endpoints
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::json;
use tokio::task;
use tracing::{debug, error, field::Empty, info, warn, Span};

use crate::{
    admin::{AdminAuthService, FirebaseAuth, FirebaseClaims},
    constants::{error_messages, limits, tiers},
    context::{AuthContext, ConfigContext, DataContext, ServerContext},
    errors::{AppError, AppResult, ErrorCode},
    mcp::resources::ServerResources,
    middleware::extract_auth_from_headers,
    models::{Tenant, TenantId, User, UserStatus, UserTier},
    permissions::UserRole,
    utils::{
        auth::extract_bearer_token_owned,
        errors::{auth_error, user_state_error, validation_error},
    },
};
use pierre_auth::security::cookies::{clear_auth_cookie, set_auth_cookie, set_csrf_cookie};

use super::types::{
    ChangePasswordRequest, CompleteResetRequest, FirebaseLoginRequest, LoginRequest, LoginResponse,
    OAuth2ErrorResponse, OAuth2TokenRequest, OAuth2TokenResponse, RefreshTokenRequest,
    RegisterRequest, RegisterResponse, SessionResponse, UpdateProfileRequest,
    UpdateProfileResponse, UserInfo, UserStatsResponse,
};

// ---------------------------------------------------------------------------
// AuthService — business logic
// ---------------------------------------------------------------------------

/// Authentication service for business logic
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

    /// Handle user registration - implementation from existing routes.rs
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
            message,
        })
    }

    /// Handle user login - implementation from existing routes.rs
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
            error!("Invalid password for login attempt");
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
                user_id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name,
                is_admin: user.is_admin,
                role: user.role.as_str().to_owned(),
                user_status: user.user_status.to_string(),
                tenant_id: tenant_id_for_response,
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
                user_id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                is_admin: user.is_admin,
                role: user.role.as_str().to_owned(),
                user_status: user.user_status.to_string(),
                tenant_id: tenant_id_for_response,
            },
        })
    }

    /// Handle token refresh - implementation from existing routes.rs
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
                user_id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name,
                is_admin: user.is_admin,
                role: user.role.as_str().to_owned(),
                user_status: user.user_status.to_string(),
                tenant_id: tenant_id_for_response,
            },
        })
    }

    /// Validate email format - from existing routes.rs
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

    /// Validate password strength - from existing routes.rs
    #[must_use]
    pub const fn is_valid_password(password: &str) -> bool {
        password.len() >= 8
    }
}

// ---------------------------------------------------------------------------
// Axum handler functions — called from AuthRoutes::routes() in mod.rs
// ---------------------------------------------------------------------------

/// Handle user registration (admin-authenticated)
///
/// REQUIRES: Admin authentication (Bearer token in Authorization header)
///
/// Security: Only administrators can create new users to prevent
/// unauthorized user creation, database pollution, and `DoS` attacks.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "admin_register", user_id = Empty, success = Empty)
)]
pub(super) async fn handle_register(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, AppError> {
    // Extract and validate admin token
    let auth_header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            AppError::auth_invalid(
                "Missing Authorization header for user registration - admin token required",
            )
        })?;

    let token = extract_bearer_token_owned(auth_header)
        .map_err(|_| AppError::auth_invalid("Invalid Authorization header format"))?;

    // Validate admin token
    let admin_auth_service = AdminAuthService::new(
        resources.repos.admin.clone(),
        resources.jwks_manager.clone(),
        resources.config.auth.admin_token_cache_ttl_secs,
    );

    // Authenticate admin (no specific permission check - any valid admin token can register users)
    admin_auth_service
        .authenticate(&token, None)
        .await
        .map_err(|e| {
            warn!(error = %e, "Failed to authenticate admin token for user registration");
            AppError::auth_invalid(format!("Admin authentication failed: {e}"))
        })?;

    info!("Admin-authenticated user registration attempt");

    let server_context = ServerContext::from(resources.as_ref());
    let auth_routes = AuthService::new(
        server_context.auth().clone(),
        server_context.config().clone(),
        server_context.data().clone(),
    );

    match auth_routes.register(request).await {
        Ok(response) => Ok((StatusCode::CREATED, Json(response)).into_response()),
        Err(e) => {
            error!("Registration failed: {}", e);
            Err(e)
        }
    }
}

/// Handle public user self-registration
///
/// This endpoint allows users to register themselves without admin authentication.
/// New users are created in "Pending" status by default and require admin approval,
/// unless `AUTO_APPROVE_USERS` is true or the email domain is in `AUTO_APPROVE_DOMAINS`.
#[tracing::instrument(
    skip(resources, request),
    fields(route = "public_register", user_id = Empty, success = Empty)
)]
pub(super) async fn handle_public_register(
    State(resources): State<Arc<ServerResources>>,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, AppError> {
    info!("Public self-registration attempt");

    let server_context = ServerContext::from(resources.as_ref());
    let auth_routes = AuthService::new(
        server_context.auth().clone(),
        server_context.config().clone(),
        server_context.data().clone(),
    );

    match auth_routes.register(request).await {
        Ok(response) => Ok((StatusCode::CREATED, Json(response)).into_response()),
        Err(e) => {
            error!("Public registration failed: {}", e);
            Err(e)
        }
    }
}

/// Handle Firebase authentication login
///
/// Authenticates users via Firebase ID tokens (Google Sign-In, Apple, etc.)
#[tracing::instrument(
    skip(resources, request),
    fields(route = "firebase_login", user_id = Empty, auth_provider = Empty, success = Empty)
)]
pub(super) async fn handle_firebase_login(
    State(resources): State<Arc<ServerResources>>,
    Json(request): Json<FirebaseLoginRequest>,
) -> Result<Response, AppError> {
    // Check if Firebase is configured
    let firebase_auth = resources.firebase_auth.as_ref().ok_or_else(|| {
        AppError::invalid_input("Firebase authentication is not configured on this server")
    })?;

    let server_context = ServerContext::from(resources.as_ref());
    let auth_service = AuthService::new(
        server_context.auth().clone(),
        server_context.config().clone(),
        server_context.data().clone(),
    );

    match auth_service
        .login_with_firebase(request, firebase_auth)
        .await
    {
        Ok(mut response) => {
            // Clone JWT for cookie (also included in JSON response for API clients)
            let jwt_token = response
                .jwt_token
                .clone() // Safe: JWT string ownership for cookie
                .ok_or_else(|| AppError::internal("JWT token missing from login response"))?;

            // Parse user ID for CSRF token generation
            let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;

            // Generate CSRF token (stateless HMAC — no server storage)
            let csrf_token = resources
                .csrf_manager
                .generate_token(user_id)
                .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

            // Set response CSRF token
            response.csrf_token.clone_from(&csrf_token);

            // Build response with secure cookies
            let mut headers = HeaderMap::new();

            // Set httpOnly auth cookie (24 hour expiry to match JWT)
            set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);

            // Set CSRF cookie (24 hour expiry to match CSRF token TTL)
            set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

            Ok((StatusCode::OK, headers, Json(response)).into_response())
        }
        Err(e) => {
            tracing::error!("Firebase login failed: {}", e);
            Err(e)
        }
    }
}

/// Handle token refresh
#[tracing::instrument(
    skip(resources, request),
    fields(route = "token_refresh", user_id = %request.user_id, success = Empty)
)]
pub(super) async fn handle_refresh(
    State(resources): State<Arc<ServerResources>>,
    Json(request): Json<RefreshTokenRequest>,
) -> Result<Response, AppError> {
    let server_context = ServerContext::from(resources.as_ref());
    let auth_service = AuthService::new(
        server_context.auth().clone(),
        server_context.config().clone(),
        server_context.data().clone(),
    );

    match auth_service.refresh_token(request).await {
        Ok(mut response) => {
            // Clone JWT for cookie (also included in JSON response for API clients)
            let jwt_token = response
                .jwt_token
                .clone() // Safe: JWT string ownership for cookie
                .ok_or_else(|| AppError::internal("JWT token missing from refresh response"))?;

            // Parse user ID for CSRF token generation
            let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;

            // Generate new CSRF token (stateless HMAC — no server storage)
            let csrf_token = resources
                .csrf_manager
                .generate_token(user_id)
                .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

            // Set response CSRF token
            response.csrf_token.clone_from(&csrf_token);

            // Build response with secure cookies
            let mut headers = HeaderMap::new();

            // Set httpOnly auth cookie (24 hour expiry to match JWT)
            set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);

            // Set CSRF cookie (24 hour expiry to match CSRF token TTL)
            set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

            Ok((StatusCode::OK, headers, Json(response)).into_response())
        }
        Err(e) => {
            error!("Token refresh failed: {}", e);
            Err(e)
        }
    }
}

/// Handle user logout
pub(super) async fn handle_logout() -> Result<Response, AppError> {
    // Yield to allow async context (required for Axum handler)
    task::yield_now().await;

    // Build response with cleared cookies
    let mut headers = HeaderMap::new();
    clear_auth_cookie(&mut headers);

    Ok((
        StatusCode::OK,
        headers,
        Json(json!({ "message": "Logged out successfully" })),
    )
        .into_response())
}

/// Restore session from httpOnly cookie authentication
///
/// Returns the authenticated user's info along with a fresh JWT (for WebSocket auth)
/// and CSRF token. This allows the frontend to restore sessions on page refresh
/// without storing JWT tokens in localStorage.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "session", user_id = Empty, success = Empty)
)]
pub(super) async fn handle_session(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using cookie or Authorization header
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    Span::current().record("user_id", user_id.to_string());

    // Look up user details from database
    let user = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

    // Preserve active_tenant_id from existing JWT, or look up user's default tenant
    let active_tenant_id = if let Some(tid) = auth_result.active_tenant_id {
        Some(tid.to_string())
    } else {
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;
        tenants.first().map(|t| t.id.to_string())
    };
    let tenant_id_for_response = active_tenant_id.clone();

    // Generate a fresh JWT token for WebSocket authentication with active_tenant_id
    let server_context = ServerContext::from(resources.as_ref());
    let jwt_token = server_context
        .auth()
        .auth_manager()
        .generate_token_with_tenant(
            &user,
            server_context.auth().jwks_manager(),
            active_tenant_id,
        )
        .map_err(|e| AppError::auth_invalid(format!("Failed to generate token: {e}")))?;

    // Generate fresh CSRF token (stateless HMAC — no server storage)
    let csrf_token = resources
        .csrf_manager
        .generate_token(user_id)
        .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

    // Refresh the httpOnly auth cookie with the new JWT
    let mut response_headers = HeaderMap::new();
    set_auth_cookie(&mut response_headers, &jwt_token, 24 * 60 * 60);
    set_csrf_cookie(&mut response_headers, &csrf_token, 24 * 60 * 60);

    Span::current().record("success", true);
    info!("Session restored for user: {}", user_id);

    let session_response = SessionResponse {
        user: UserInfo {
            user_id: user.id.to_string(),
            email: user.email.clone(),
            display_name: user.display_name,
            is_admin: user.is_admin,
            role: user.role.as_str().to_owned(),
            user_status: user.user_status.to_string(),
            tenant_id: tenant_id_for_response,
        },
        access_token: jwt_token,
        csrf_token,
    };

    Ok((StatusCode::OK, response_headers, Json(session_response)).into_response())
}

/// Handle user profile update
///
/// Updates the authenticated user's display name.
/// Requires valid JWT authentication via cookie or Bearer token.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "update_profile", success = Empty)
)]
pub(super) async fn handle_update_profile(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Response, AppError> {
    // Authenticate and get user ID
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    // Validate display name
    let display_name = request.display_name.trim();
    if display_name.is_empty() {
        return Err(AppError::invalid_input("Display name cannot be empty"));
    }
    if display_name.len() > 100 {
        return Err(AppError::invalid_input(
            "Display name must be 100 characters or less",
        ));
    }

    // Update user in database
    let updated_user = resources
        .repos
        .users
        .update_display_name(user_id, display_name)
        .await?;

    // Build response
    let response = UpdateProfileResponse {
        message: "Profile updated successfully".to_owned(),
        user: UserInfo {
            user_id: updated_user.id.to_string(),
            email: updated_user.email,
            display_name: updated_user.display_name,
            is_admin: updated_user.is_admin,
            role: updated_user.role.to_string(),
            user_status: updated_user.user_status.to_string(),
            tenant_id: None,
        },
    };

    info!(user_id = %user_id, "User profile updated successfully");

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle password change for authenticated users
///
/// Verifies the current password, validates the new password,
/// then hashes and stores the new password.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "change_password", success = Empty)
)]
pub(super) async fn handle_change_password(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Response, AppError> {
    // Authenticate and get user ID
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    // Fetch user to get current password hash
    let user = resources
        .repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

    // Verify current password using spawn_blocking to avoid blocking async executor
    let current_password = request.current_password;
    let stored_hash = user.password_hash.clone();
    let is_valid = task::spawn_blocking(move || bcrypt::verify(&current_password, &stored_hash))
        .await
        .map_err(|e| AppError::internal(format!("Password verification task failed: {e}")))?
        .map_err(|_| AppError::auth_invalid("Current password is incorrect"))?;

    if !is_valid {
        return Err(AppError::auth_invalid("Current password is incorrect"));
    }

    // Validate new password strength
    if !AuthService::is_valid_password(&request.new_password) {
        return Err(AppError::invalid_input(error_messages::PASSWORD_TOO_WEAK));
    }

    // Hash new password using spawn_blocking
    let password_to_hash = request.new_password;
    let password_hash =
        task::spawn_blocking(move || bcrypt::hash(&password_to_hash, bcrypt::DEFAULT_COST))
            .await
            .map_err(|e| AppError::internal(format!("Password hashing task failed: {e}")))?
            .map_err(|e| AppError::internal(format!("Password hashing failed: {e}")))?;

    // Update password in database
    resources
        .repos
        .users
        .update_password(user_id, &password_hash)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user password");
            AppError::internal(format!("Failed to update password: {e}"))
        })?;

    Span::current().record("success", true);
    info!(user_id = %user_id, "User password changed successfully");

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Password changed successfully" })),
    )
        .into_response())
}

/// Complete a password reset using a one-time token
///
/// Public endpoint — no authentication required. The reset token acts as proof
/// of authorization (issued by an admin). The user provides the token and their
/// chosen new password. The token is consumed atomically to prevent replay.
#[tracing::instrument(skip(resources, request), fields(route = "complete_reset"))]
pub(super) async fn handle_complete_reset(
    State(resources): State<Arc<ServerResources>>,
    Json(request): Json<CompleteResetRequest>,
) -> Result<Response, AppError> {
    use sha2::{Digest, Sha256};

    // Validate new password strength
    if !AuthService::is_valid_password(&request.new_password) {
        return Err(AppError::invalid_input(error_messages::PASSWORD_TOO_WEAK));
    }

    // Hash the presented token to match against stored hash
    let token_hash = format!("{:x}", Sha256::digest(request.reset_token.as_bytes()));

    // Atomically consume the token (validates existence, expiry, and single-use)
    let user_id = resources
        .repos
        .password_reset
        .consume_token(&token_hash)
        .await?;

    // Hash the new password using spawn_blocking to avoid blocking async executor
    let password_to_hash = request.new_password;
    let password_hash =
        task::spawn_blocking(move || bcrypt::hash(&password_to_hash, bcrypt::DEFAULT_COST))
            .await
            .map_err(|e| AppError::internal(format!("Password hashing task failed: {e}")))?
            .map_err(|e| AppError::internal(format!("Password hashing failed: {e}")))?;

    // Update the user's password
    resources
        .repos
        .users
        .update_password(user_id, &password_hash)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user password during reset");
            AppError::internal(format!("Failed to update password: {e}"))
        })?;

    // Invalidate any other outstanding reset tokens for this user
    resources
        .repos
        .password_reset
        .invalidate_tokens(user_id)
        .await
        .map_err(|e| {
            warn!(error = %e, "Failed to invalidate remaining reset tokens");
            AppError::internal(format!("Failed to cleanup reset tokens: {e}"))
        })?;

    info!(user_id = %user_id, "Password reset completed via one-time token");

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Password has been reset successfully" })),
    )
        .into_response())
}

/// Handle self-service forgot-password requests
///
/// Generates a 6-digit numeric code, stores its SHA-256 hash with a 15-minute TTL,
/// and sends it to the user via email (if Resend is configured).
///
/// Security: Always returns HTTP 200 with an identical message regardless of whether
/// the email exists, to prevent account enumeration attacks.
#[tracing::instrument(skip(resources, request), fields(route = "forgot_password"))]
pub(super) async fn handle_forgot_password(
    State(resources): State<Arc<ServerResources>>,
    Json(request): Json<super::types::ForgotPasswordRequest>,
) -> Result<Response, AppError> {
    use crate::constants::password_reset;
    use rand::Rng;
    use sha2::{Digest, Sha256};

    let anti_enum_message =
        "If an account with that email exists, a reset code has been sent.".to_owned();

    // Basic email format validation
    if !request.email.contains('@') || request.email.len() < 5 {
        return Err(AppError::invalid_input("Invalid email format"));
    }

    // Look up user — if not found, return the same success message (anti-enumeration)
    let user = resources.repos.users.get_by_email(&request.email).await?;

    let Some(user) = user else {
        info!("Forgot password requested for nonexistent email (anti-enumeration)");
        return Ok((
            StatusCode::OK,
            Json(super::types::ForgotPasswordResponse {
                message: anti_enum_message,
            }),
        )
            .into_response());
    };

    // Rate limit: max N codes per hour per user
    let one_hour_ago = Utc::now() - chrono::Duration::hours(1);
    let recent_count = resources
        .repos
        .password_reset
        .count_recent_tokens(user.id, one_hour_ago)
        .await?;

    if recent_count >= password_reset::MAX_CODES_PER_HOUR {
        info!(
            user_id = %user.id,
            recent_count = recent_count,
            "Rate limit reached for password reset codes — skipping silently"
        );
        return Ok((
            StatusCode::OK,
            Json(super::types::ForgotPasswordResponse {
                message: anti_enum_message,
            }),
        )
            .into_response());
    }

    // Generate 6-digit numeric code
    let code: u32 = rand::thread_rng()
        .gen_range(password_reset::CODE_RANGE_MIN..password_reset::CODE_RANGE_MAX);
    let code_str = code.to_string();

    // Hash the code before storing (same SHA-256 approach as admin tokens)
    let code_hash = format!("{:x}", Sha256::digest(code_str.as_bytes()));

    // Store with short TTL
    resources
        .repos
        .password_reset
        .store_token_with_ttl(
            user.id,
            &code_hash,
            password_reset::CREATED_BY_SELF_SERVICE,
            password_reset::CODE_TTL_MINUTES,
        )
        .await?;

    // Send email (or log warning if Resend not configured)
    if let Some(email_svc) = &resources.email_service {
        if let Err(e) = email_svc
            .send_password_reset_code(&request.email, &code_str)
            .await
        {
            warn!(error = %e, "Failed to send password reset email — user will not receive code");
        }
    } else {
        warn!("Email service not configured — password reset code generated but not delivered");
    }

    info!(user_id = %user.id, "Password reset code issued via self-service flow");

    Ok((
        StatusCode::OK,
        Json(super::types::ForgotPasswordResponse {
            message: anti_enum_message,
        }),
    )
        .into_response())
}

/// Handle user stats request for dashboard
///
/// Returns aggregated stats: connected providers, activities synced, and days active.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "user_stats", user_id = Empty)
)]
pub(super) async fn handle_user_stats(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate and get user ID
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;
    Span::current().record("user_id", user_id.to_string());

    // Get connected providers count from OAuth tokens (cross-tenant view for user stats)
    let oauth_tokens = resources
        .repos
        .oauth_tokens
        .get_tokens(user_id, None)
        .await?;
    let connected_providers = i64::try_from(oauth_tokens.len()).unwrap_or(0);

    // Get user creation date to calculate days active
    let user = resources.repos.users.get_global(user_id).await?;
    let days_active = match user {
        Some(u) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(u.created_at);
            duration.num_days().max(1)
        }
        None => 1,
    };

    let response = UserStatsResponse {
        connected_providers,
        days_active,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle `OAuth2` ROPC (Resource Owner Password Credentials) token request
///
/// This endpoint implements RFC 6749 Section 4.3 for MCP and CLI clients
/// that need to obtain tokens without a browser-based OAuth flow.
///
/// Request format: `application/x-www-form-urlencoded`
/// ```text
/// grant_type=password&username=user@example.com&password=secret
/// ```
///
/// Response format: RFC 6749 Section 5.1 compliant JSON
#[tracing::instrument(
    skip(resources, request),
    fields(
        route = "oauth2_token",
        grant_type = %request.grant_type,
        username = %request.username,
        user_id = Empty,
        success = Empty,
    )
)]
pub(super) async fn handle_oauth2_token(
    State(resources): State<Arc<ServerResources>>,
    Form(request): Form<OAuth2TokenRequest>,
) -> Result<Response, AppError> {
    // Validate grant_type
    if request.grant_type != "password" {
        let error_response = OAuth2ErrorResponse {
            error: "unsupported_grant_type".to_owned(),
            error_description: Some(format!(
                "Grant type '{}' is not supported. Use 'password' for ROPC.",
                request.grant_type
            )),
        };
        return Ok((StatusCode::BAD_REQUEST, Json(error_response)).into_response());
    }

    // Delegate to existing login logic
    let login_request = LoginRequest {
        email: request.username,
        password: request.password,
    };

    let server_context = ServerContext::from(resources.as_ref());
    let auth_service = AuthService::new(
        server_context.auth().clone(),
        server_context.config().clone(),
        server_context.data().clone(),
    );

    match auth_service.login(login_request).await {
        Ok(response) => {
            let jwt_token = response
                .jwt_token
                .clone()
                .ok_or_else(|| AppError::internal("JWT token missing from login response"))?;

            // Parse expiration to calculate expires_in
            let expires_at = chrono::DateTime::parse_from_rfc3339(&response.expires_at)
                .map_or_else(
                    |_| chrono::Utc::now() + chrono::Duration::hours(24),
                    |dt| dt.with_timezone(&chrono::Utc),
                );
            let expires_in = (expires_at - chrono::Utc::now()).num_seconds();

            // Generate CSRF token for web clients (stateless HMAC — no server storage)
            let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;
            let csrf_token = resources
                .csrf_manager
                .generate_token(user_id)
                .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

            let oauth2_response = OAuth2TokenResponse {
                access_token: jwt_token.clone(),
                token_type: "Bearer".to_owned(),
                expires_in,
                refresh_token: None,
                scope: request.scope,
                // Pierre extensions for frontend compatibility
                user: Some(response.user),
                csrf_token: Some(csrf_token.clone()),
            };

            // Build response with secure cookies for web clients
            let mut headers = HeaderMap::new();
            set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);
            set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

            Ok((StatusCode::OK, headers, Json(oauth2_response)).into_response())
        }
        Err(e) => {
            // Map to OAuth2 error format based on error code
            let error_code = match e.code {
                ErrorCode::AuthInvalid | ErrorCode::AuthRequired | ErrorCode::AuthExpired => {
                    "invalid_grant"
                }
                ErrorCode::PermissionDenied
                | ErrorCode::AccountPending
                | ErrorCode::AccountSuspended => "access_denied",
                ErrorCode::InvalidInput | ErrorCode::InvalidFormat => "invalid_request",
                _ => "server_error",
            };
            let error_desc = e.message;

            let error_response = OAuth2ErrorResponse {
                error: error_code.to_owned(),
                error_description: Some(error_desc),
            };

            // OAuth2 spec: invalid_grant returns 400, server_error returns 500
            let status = if error_code == "server_error" {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::BAD_REQUEST
            };

            Ok((status, Json(error_response)).into_response())
        }
    }
}
