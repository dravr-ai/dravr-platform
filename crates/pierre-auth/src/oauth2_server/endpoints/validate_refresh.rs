// ABOUTME: Access-token validation with optional refresh for the OAuth 2.0 authorization server
// ABOUTME: Checks a JWT, and when it has expired rotates the presented refresh token into a fresh pair
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use jsonwebtoken::dangerous::insecure_decode;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::OAuth2AuthorizationServer;
use crate::auth::{Claims, JwtValidationError};
use crate::oauth2_server::models::{
    OAuth2RefreshToken, ValidateRefreshRequest, ValidateRefreshResponse, ValidationStatus,
};
use crate::oauth2_server::resource::token_audience;

impl OAuth2AuthorizationServer {
    /// Validate and optionally refresh an access token
    ///
    /// This endpoint checks if a JWT access token is valid. If valid, it returns the expiration time.
    /// If expired but a refresh token is provided, it attempts to refresh and return new tokens.
    /// If invalid or cannot be refreshed, it returns an error with the reason.
    ///
    /// # Errors
    /// Returns an error if token validation fails catastrophically (database errors, etc.)
    pub async fn validate_and_refresh(
        &self,
        access_token: &str,
        request: ValidateRefreshRequest,
    ) -> AppResult<ValidateRefreshResponse> {
        // Validate the JWT token: the platform audience, or a token bound to
        // the resource this server mints for
        match self.auth_manager.validate_resource_token_detailed(
            access_token,
            &self.jwks_manager,
            &self.served(),
        ) {
            Ok(claims) => self.handle_valid_token_claims(claims).await,
            Err(validation_error) => {
                self.handle_token_validation_error(validation_error, access_token, &request)
                    .await
            }
        }
    }

    /// Handle valid token claims by checking user existence
    async fn handle_valid_token_claims(
        &self,
        claims: Claims,
    ) -> AppResult<ValidateRefreshResponse> {
        match Uuid::parse_str(&claims.sub) {
            // SECURITY: Global lookup — OAuth2 token validation, no tenant context
            Ok(user_id) => match self.users.get_global(user_id).await {
                Ok(Some(_user)) => Ok(ValidateRefreshResponse {
                    status: ValidationStatus::Valid,
                    expires_in: Some(claims.exp - Utc::now().timestamp()),
                    access_token: None,
                    refresh_token: None,
                    token_type: None,
                    reason: None,
                    requires_full_reauth: None,
                }),
                Ok(None) => Ok(Self::create_invalid_response("user_not_found")),
                Err(e) => {
                    error!("Database error while validating token: {}", e);
                    Ok(Self::create_invalid_response("database_error"))
                }
            },
            Err(_) => Ok(Self::create_invalid_response("invalid_user_id")),
        }
    }

    /// Create a successful refresh response
    fn create_refreshed_response(
        new_access_token: String,
        refresh_token_value: &str,
    ) -> ValidateRefreshResponse {
        ValidateRefreshResponse {
            status: ValidationStatus::Refreshed,
            expires_in: Some(3600), // 1 hour
            access_token: Some(new_access_token),
            refresh_token: Some(refresh_token_value.to_owned()),
            token_type: Some("Bearer".to_owned()),
            reason: None,
            requires_full_reauth: None,
        }
    }

    /// Execute the refresh token rotation and access token generation
    ///
    /// Uses `?` propagation — caller converts errors to invalid responses.
    async fn execute_token_refresh(
        &self,
        refresh_token_value: &str,
        user_id_str: &str,
    ) -> AppResult<ValidateRefreshResponse> {
        let refresh_token_data = self
            .lookup_and_validate_refresh_token(refresh_token_value, user_id_str)
            .await?;

        // Checked before the refresh token is rotated away: a grant bound to a
        // resource this server no longer serves cannot mint a usable token.
        let audience = token_audience(&self.served(), None, refresh_token_data.resource.as_deref())
            .map_err(|refusal| {
                AppError::new(
                    ErrorCode::AuthInvalid,
                    refusal.error_description.unwrap_or(refusal.error),
                )
            })?;

        let (_, new_refresh_token_value) = self
            .rotate_refresh_token(refresh_token_value, &refresh_token_data.client_id)
            .await?
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::AuthInvalid,
                    "Refresh token already consumed (possible replay)",
                )
            })?;

        let new_access_token = self
            .generate_access_token(
                &refresh_token_data.client_id,
                Some(refresh_token_data.user_id),
                &Self::delegated_grant(refresh_token_data.scope.as_deref()),
                audience.as_deref(),
            )
            .await?;

        info!(
            "Refresh token rotated via validate_and_refresh for user {}",
            user_id_str
        );

        Ok(Self::create_refreshed_response(
            new_access_token,
            &new_refresh_token_value,
        ))
    }

    /// Attempt to refresh an expired token using a refresh token
    ///
    /// Enforces the same token lifecycle as `handle_refresh_token_grant`:
    /// atomically consumes the old refresh token and issues a rotated one.
    async fn attempt_token_refresh(
        &self,
        refresh_token_value: &str,
        claims: &Claims,
    ) -> AppResult<ValidateRefreshResponse> {
        match self
            .execute_token_refresh(refresh_token_value, &claims.sub)
            .await
        {
            Ok(response) => Ok(response),
            Err(e) => {
                // A spent, revoked or expired refresh token is the caller's; the
                // lookup or the minting failing is an outage and pages.
                if e.is_server_fault() {
                    error!("Token refresh failed: {}", e);
                } else {
                    warn!("Token refresh failed: {}", e);
                }
                Ok(Self::create_invalid_response("invalid_refresh_token"))
            }
        }
    }

    /// Handle expired token with optional refresh
    async fn handle_expired_token(
        &self,
        expired_access_token: &str,
        refresh_token_value: Option<&String>,
    ) -> AppResult<ValidateRefreshResponse> {
        let Some(refresh_token_value) = refresh_token_value else {
            return Ok(Self::create_invalid_response("token_expired"));
        };

        info!("Access token expired, attempting refresh with provided refresh_token");

        // Decode expired token to extract user_id and client info (without validation)
        let claims = match Self::decode_expired_token(expired_access_token) {
            Ok(claims) => claims,
            Err(e) => {
                error!("Failed to decode expired token: {}", e);
                return Ok(Self::create_invalid_response("malformed_expired_token"));
            }
        };

        self.attempt_token_refresh(refresh_token_value, &claims)
            .await
    }

    /// Handle JWT validation errors
    async fn handle_token_validation_error(
        &self,
        validation_error: JwtValidationError,
        expired_access_token: &str,
        request: &ValidateRefreshRequest,
    ) -> AppResult<ValidateRefreshResponse> {
        match validation_error {
            JwtValidationError::TokenExpired { .. } => {
                self.handle_expired_token(expired_access_token, request.refresh_token.as_ref())
                    .await
            }
            JwtValidationError::TokenInvalid { reason } => {
                debug!("Token invalid: {reason}");
                Ok(Self::create_invalid_response("invalid_signature"))
            }
            JwtValidationError::TokenMalformed { details } => {
                debug!("Token malformed: {details}");
                Ok(Self::create_invalid_response("malformed_token"))
            }
        }
    }

    /// Create an invalid token response
    fn create_invalid_response(reason: &str) -> ValidateRefreshResponse {
        ValidateRefreshResponse {
            status: ValidationStatus::Invalid,
            expires_in: None,
            access_token: None,
            refresh_token: None,
            token_type: None,
            reason: Some(reason.to_owned()),
            requires_full_reauth: Some(true),
        }
    }

    /// Decode an expired JWT token without validation to extract claims
    ///
    /// This is safe because we only need to read the claims, not trust them.
    /// The refresh token will be validated separately.
    fn decode_expired_token(token: &str) -> AppResult<Claims> {
        // Decode without validation - we only need the claims data.
        // The refresh token will be validated separately.
        let token_data = insecure_decode::<Claims>(token).map_err(|e| {
            AppError::new(
                ErrorCode::AuthMalformed,
                format!("Failed to decode expired token: {e}"),
            )
        })?;

        Ok(token_data.claims)
    }

    /// Look up refresh token by value and validate it belongs to the specified user
    async fn lookup_and_validate_refresh_token(
        &self,
        refresh_token_value: &str,
        user_id_str: &str,
    ) -> AppResult<OAuth2RefreshToken> {
        // Parse user_id from string
        let user_id = Uuid::parse_str(user_id_str).map_err(|e| {
            AppError::new(
                ErrorCode::AuthMalformed,
                format!("Invalid user_id in token claims: {e}"),
            )
        })?;

        // Look up refresh token in database
        // We need to find it without knowing the client_id
        let refresh_token = self
            .oauth2_server
            .get_refresh_token_by_value(refresh_token_value)
            .await
            .map_err(|e| {
                AppError::new(
                    ErrorCode::DatabaseError,
                    format!("Database error looking up refresh token: {e}"),
                )
            })?
            .ok_or_else(|| AppError::new(ErrorCode::ResourceNotFound, "Refresh token not found"))?;

        // Verify the refresh token belongs to this user
        if refresh_token.user_id != user_id {
            return Err(AppError::new(
                ErrorCode::AuthInvalid,
                "Refresh token does not belong to the user in the access token",
            ));
        }

        // Verify the refresh token hasn't expired
        if refresh_token.expires_at < Utc::now() {
            return Err(AppError::new(
                ErrorCode::AuthExpired,
                "Refresh token has expired",
            ));
        }

        // Verify the refresh token hasn't been revoked
        if refresh_token.revoked {
            return Err(AppError::new(
                ErrorCode::AuthInvalid,
                "Refresh token has been revoked",
            ));
        }

        Ok(refresh_token)
    }
}
