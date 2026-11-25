// ABOUTME: Access-token validation with optional refresh for the OAuth 2.0 authorization server
// ABOUTME: Checks a JWT, and when it has expired rotates the presented refresh token into a fresh pair
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use jsonwebtoken::dangerous::insecure_decode;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use secrecy::{ExposeSecret, SecretString};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::admin_grant::holds_admin;
use super::OAuth2AuthorizationServer;
use crate::auth::{oauth_access_token_lifetime_secs, Claims, JwtValidationError, TokenLineage};
use crate::oauth2_server::first_party::is_first_party;
use crate::oauth2_server::models::{
    OAuth2Error, OAuth2RefreshToken, ValidateRefreshRequest, ValidateRefreshResponse,
    ValidationStatus,
};

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
        // any resource this server mints for, a satellite's included
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
                Ok(Some(_user)) => match self.access_was_withdrawn(user_id, &claims).await {
                    Ok(false) => Ok(ValidateRefreshResponse {
                        status: ValidationStatus::Valid,
                        expires_in: Some(claims.exp - Timestamp::now().epoch_seconds()),
                        access_token: None,
                        refresh_token: None,
                        token_type: None,
                        reason: None,
                        requires_full_reauth: None,
                        retry_after: None,
                    }),
                    Ok(true) => Ok(Self::create_invalid_response("grant_revoked")),
                    Err(e) => {
                        error!("Database error while validating token: {}", e);
                        Ok(Self::create_invalid_response("database_error"))
                    }
                },
                Ok(None) => Ok(Self::create_invalid_response("user_not_found")),
                Err(e) => {
                    error!("Database error while validating token: {}", e);
                    Ok(Self::create_invalid_response("database_error"))
                }
            },
            Err(_) => Ok(Self::create_invalid_response("invalid_user_id")),
        }
    }

    /// Whether the athlete withdrew the consent a delegated token rides on, as
    /// `/mcp` and A2A would refuse it. This route is not on the path of every
    /// call, so it reads the grant itself. A session ([`Claims::is_session`])
    /// rides on no grant; a token an application holds does, however wide.
    async fn access_was_withdrawn(&self, user_id: Uuid, claims: &Claims) -> AppResult<bool> {
        let (Some(client_id), Some(tenant_id)) = (&claims.client_id, &claims.active_tenant_id)
        else {
            return Ok(false);
        };
        if claims.is_session() {
            return Ok(false);
        }
        let standing = self
            .oauth2_server
            .client_grant_standing(&user_id.to_string(), tenant_id, client_id)
            .await?;
        Ok(!standing.admits_token(
            claims.iat,
            claims.client_grant_id.as_deref(),
            Timestamp::now(),
        ))
    }

    /// Create a successful refresh response
    fn create_refreshed_response(
        new_access_token: String,
        refresh_token_value: &str,
        client_id: &str,
    ) -> ValidateRefreshResponse {
        ValidateRefreshResponse {
            status: ValidationStatus::Refreshed,
            expires_in: Some(oauth_access_token_lifetime_secs(client_id)),
            access_token: Some(SecretString::from(new_access_token)),
            refresh_token: Some(SecretString::from(refresh_token_value.to_owned())),
            token_type: Some("Bearer".to_owned()),
            reason: None,
            requires_full_reauth: None,
            retry_after: None,
        }
    }

    /// The answer to a refresh its user's window refused, or could not count.
    /// It never reads as a dead token: nothing was rotated and the client keeps
    /// what it holds.
    fn create_retry_later_response(refusal: OAuth2Error) -> ValidateRefreshResponse {
        ValidateRefreshResponse {
            status: ValidationStatus::RetryLater,
            expires_in: None,
            access_token: None,
            refresh_token: None,
            token_type: None,
            reason: Some(refusal.error),
            requires_full_reauth: Some(false),
            retry_after: refusal.retry_after,
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
        if let Some(refused) = Self::refused_here(&refresh_token_data) {
            return Ok(refused);
        }

        // Checked before the refresh token is rotated away: a grant bound to a
        // resource this server no longer serves cannot mint a usable token,
        // and a grant whose tenant the user left is revoked.
        // `/oauth2/validate-and-refresh` answers no details: the SDK reads
        // the token and nothing beside it.
        let (grant, _) = self
            .check_refresh_grant(refresh_token_value, None)
            .await
            .map_err(refusal_error)?;
        let audience = self
            .audience(None, refresh_token_data.resource.as_deref())
            .map_err(refusal_error)?;

        // The same window `/oauth2/token` counts a refresh in, before the token
        // is rotated away, so this route is no way around it.
        if let Err(refusal) = self
            .admit_refresh_user(grant.as_ref(), &refresh_token_data.client_id)
            .await
        {
            return Ok(Self::create_retry_later_response(refusal));
        }

        let (consumed, new_refresh_token_value, granted) = self
            .rotate_refresh_token(refresh_token_value, &refresh_token_data.client_id)
            .await?
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::AuthInvalid,
                    "Refresh token already consumed (possible replay)",
                )
            })?;

        let new_access_token = self.generate_access_token(
            &refresh_token_data.client_id,
            Some((
                refresh_token_data.user_id,
                refresh_token_data.tenant_id.as_str(),
            )),
            Some(TokenLineage {
                family_id: &consumed.family_id,
                client_grant_id: consumed.client_grant_id.as_deref(),
                // A bound family never reaches here (`refused_here`).
                dpop_jkt: None,
            }),
            &granted,
            audience.as_deref(),
        )?;

        info!(
            "Refresh token rotated via validate_and_refresh for user {}",
            user_id_str
        );

        Ok(Self::create_refreshed_response(
            new_access_token,
            &new_refresh_token_value,
            &refresh_token_data.client_id,
        ))
    }

    /// The answer to a refresh this route does not make, before anything is
    /// rotated; `None` for one it makes.
    ///
    /// This route rotates without authenticating the client, so it keeps to
    /// grants a leaked refresh token can do least with. A grant holding
    /// `admin` refreshes at `/oauth2/token`, which authenticates the client
    /// (carnet#1511, review M3); nothing is rotated here, so the token stays
    /// good there. Dravr's own apps hold no OAuth refresh token at all: one
    /// naming them would mint a token that reads as a session.
    fn refused_here(refresh_token: &OAuth2RefreshToken) -> Option<ValidateRefreshResponse> {
        if holds_admin(refresh_token.scope.as_deref()) {
            warn!(user_id = %refresh_token.user_id, client_id = %refresh_token.client_id, "validate-and-refresh refused an admin grant; it refreshes at /oauth2/token");
            return Some(Self::create_token_endpoint_response());
        }
        // A family bound to a `DPoP` key rotates only with a proof by it,
        // which this route has no way to carry: it refreshes at
        // `/oauth2/token`, where nothing here has rotated it away.
        if refresh_token.dpop_jkt.is_some() {
            warn!(user_id = %refresh_token.user_id, client_id = %refresh_token.client_id, "validate-and-refresh refused a DPoP-bound grant; it refreshes at /oauth2/token");
            return Some(Self::create_token_endpoint_response());
        }
        if is_first_party(&refresh_token.client_id) {
            warn!(client_id = %refresh_token.client_id, "validate-and-refresh refused a refresh token naming a first-party client");
            return Some(Self::create_invalid_response("invalid_refresh_token"));
        }
        None
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
        refresh_token_value: Option<&str>,
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
                self.handle_expired_token(
                    expired_access_token,
                    request
                        .refresh_token
                        .as_ref()
                        .map(ExposeSecret::expose_secret),
                )
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

    /// The answer to a refresh this route does not make: the grant holds
    /// `admin`, which refreshes only where the client authenticates. Nothing
    /// was rotated, so the client keeps its refresh token and is not told to
    /// send its athlete through consent again.
    fn create_token_endpoint_response() -> ValidateRefreshResponse {
        ValidateRefreshResponse {
            requires_full_reauth: Some(false),
            ..Self::create_invalid_response(ADMIN_REFRESHES_AT_TOKEN_ENDPOINT)
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
            retry_after: None,
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
        if refresh_token.expires_at < Timestamp::now() {
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

/// The reason `validate_and_refresh` gives for a grant holding `admin`: it
/// refreshes at `/oauth2/token`, where the client authenticates.
pub const ADMIN_REFRESHES_AT_TOKEN_ENDPOINT: &str = "refresh_at_token_endpoint";

/// A token-endpoint refusal as the error `validate_and_refresh` answers with:
/// the server failing stays an internal error, anything else refuses the
/// token.
fn refusal_error(refusal: OAuth2Error) -> AppError {
    let message = refusal
        .error_description
        .unwrap_or_else(|| refusal.error.clone());
    if refusal.error == "server_error" {
        AppError::internal(message)
    } else {
        AppError::new(ErrorCode::AuthInvalid, message)
    }
}
