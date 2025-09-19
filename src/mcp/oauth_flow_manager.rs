// ABOUTME: OAuth flow management for multi-tenant MCP server
// ABOUTME: Handles authorization requests, callbacks, token processing, and template rendering

use super::resources::ServerResources;
use crate::database_plugins::{factory::Database, DatabaseProvider};
use crate::tenant::{TenantContext, TenantOAuthClient, TenantRole};
use crate::utils::json_responses::{api_error, oauth_error};
use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;
use warp::http::HeaderMap;

/// Manages OAuth authorization flows for multiple providers and tenants
pub struct OAuthFlowManager {
    resources: Arc<ServerResources>,
}

impl OAuthFlowManager {
    /// Create a new OAuth flow manager
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    /// Handle OAuth authorization request for a specific provider
    pub async fn handle_authorization_request(
        &self,
        provider: String,
        user_id_str: String,
        headers: HeaderMap,
    ) -> Result<impl warp::Reply, warp::Rejection> {
        let user_id = Uuid::parse_str(&user_id_str)
            .map_err(|_| warp::reject::custom(ApiError(api_error("Invalid user ID format"))))?;

        let user = self.get_user_with_tenant(user_id).await?;
        let tenant_id = self.extract_tenant_id(&user)?;

        self.process_oauth_credentials(&headers, &provider, tenant_id, user_id)
            .await?;

        let tenant_name = self.get_tenant_name(tenant_id).await;
        let tenant_context = TenantContext {
            tenant_id,
            user_id,
            tenant_name,
            user_role: TenantRole::Member,
        };

        self.generate_authorization_redirect(&provider, &tenant_context)
            .await
    }

    /// Process OAuth credentials from request headers
    async fn process_oauth_credentials(
        &self,
        headers: &HeaderMap,
        provider: &str,
        tenant_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), warp::Rejection> {
        // Extract OAuth credentials from headers
        let client_id = headers
            .get("x-oauth-client-id")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| {
                warp::reject::custom(ApiError(api_error("Missing OAuth client ID in headers")))
            })?;

        let client_secret = headers
            .get("x-oauth-client-secret")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| {
                warp::reject::custom(ApiError(api_error(
                    "Missing OAuth client secret in headers",
                )))
            })?;

        let redirect_uri = headers
            .get("x-oauth-redirect-uri")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_else(|| self.get_provider_redirect_uri(provider));

        // Store MCP OAuth credentials for this tenant
        self.store_mcp_oauth_credentials(tenant_id, user_id, provider, client_id, client_secret, redirect_uri)
            .await
            .map_err(|e| {
                error!("Failed to store OAuth credentials: {}", e);
                warp::reject::custom(ApiError(oauth_error("Failed to store OAuth credentials")))
            })?;

        Ok(())
    }

    /// Generate OAuth authorization redirect URL
    async fn generate_authorization_redirect(
        &self,
        provider: &str,
        tenant_context: &TenantContext,
    ) -> Result<impl warp::Reply, warp::Rejection> {
        let provider_registry = &self.resources.provider_registry;

        match provider_registry
            .get_provider(provider)
            .and_then(|p| p.get_authorization_url(tenant_context))
            .await
        {
            Ok(auth_url) => {
                info!(
                    "Generated authorization URL for provider {} and tenant {}: {}",
                    provider, tenant_context.tenant_id, auth_url
                );

                // Return redirect response
                Ok(warp::redirect::temporary(
                    auth_url.parse().map_err(|_| {
                        warp::reject::custom(ApiError(oauth_error("Invalid authorization URL")))
                    })?,
                ))
            }
            Err(e) => {
                error!(
                    "Failed to generate authorization URL for provider {}: {}",
                    provider, e
                );
                Err(warp::reject::custom(ApiError(oauth_error(
                    "Failed to generate authorization URL",
                ))))
            }
        }
    }

    /// Store MCP OAuth credentials for a tenant
    async fn store_mcp_oauth_credentials(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<()> {
        let credential = crate::models::TenantOAuthCredential {
            id: Uuid::new_v4(),
            tenant_id,
            user_id,
            provider: provider.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            redirect_uri: redirect_uri.to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.resources
            .database
            .store_tenant_oauth_credential(&credential)
            .await?;

        info!(
            "Stored OAuth credentials for tenant {} with provider {}",
            tenant_id, provider
        );

        Ok(())
    }

    /// Get user with tenant information
    async fn get_user_with_tenant(
        &self,
        user_id: Uuid,
    ) -> Result<crate::models::User, warp::Rejection> {
        self.resources
            .database
            .get_user_by_id(user_id)
            .await
            .map_err(|e| {
                error!("Failed to get user {}: {}", user_id, e);
                warp::reject::custom(ApiError(api_error("User not found")))
            })?
            .ok_or_else(|| warp::reject::custom(ApiError(api_error("User not found"))))
    }

    /// Extract tenant ID from user
    fn extract_tenant_id(&self, user: &crate::models::User) -> Result<Uuid, warp::Rejection> {
        user.tenant_id.ok_or_else(|| {
            warp::reject::custom(ApiError(api_error(
                "User does not belong to any tenant",
            )))
        })
    }

    /// Get tenant name
    async fn get_tenant_name(&self, tenant_id: Uuid) -> String {
        match self.resources.database.get_tenant_by_id(tenant_id).await {
            Ok(Some(tenant)) => tenant.name,
            Ok(None) => {
                warn!("Tenant {} not found, using default name", tenant_id);
                "Unknown Tenant".to_string()
            }
            Err(e) => {
                warn!("Failed to get tenant {}: {}, using default name", tenant_id, e);
                "Unknown Tenant".to_string()
            }
        }
    }

    /// Get provider redirect URI
    fn get_provider_redirect_uri(&self, provider: &str) -> &'static str {
        match provider {
            "strava" => "http://localhost:8080/api/oauth/callback/strava",
            "fitbit" => "http://localhost:8080/api/oauth/callback/fitbit",
            _ => "http://localhost:8080/api/oauth/callback/unknown",
        }
    }
}

/// Template renderer for OAuth success and error pages
pub struct OAuthTemplateRenderer;

impl OAuthTemplateRenderer {
    /// Render OAuth success template
    pub fn render_success_template(
        provider: &str,
        callback_response: &crate::routes::OAuthCallbackResponse,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let template = format!(
            r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OAuth Success - {}</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 40px; background-color: #f5f5f5; }}
        .container {{ max-width: 600px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        .success {{ color: #27ae60; font-size: 24px; margin-bottom: 20px; }}
        .info {{ color: #2c3e50; margin: 10px 0; }}
        .code {{ background: #ecf0f1; padding: 10px; border-radius: 4px; font-family: monospace; }}
    </style>
</head>
<body>
    <div class="container">
        <h1 class="success">✓ OAuth Authorization Successful</h1>
        <div class="info"><strong>Provider:</strong> {}</div>
        <div class="info"><strong>Status:</strong> Connected successfully</div>
        <div class="info"><strong>User ID:</strong> {}</div>
        <div class="info"><strong>Access Token:</strong> <span class="code">{}...</span></div>
        <p>You can now close this window and return to your application.</p>
    </div>
</body>
</html>
"#,
            provider,
            provider,
            callback_response.user_id,
            callback_response.access_token.chars().take(20).collect::<String>()
        );

        Ok(template)
    }

    /// Render OAuth error template
    pub fn render_error_template(
        provider: &str,
        error: &str,
        description: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let template = format!(
            r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OAuth Error - {}</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 40px; background-color: #f5f5f5; }}
        .container {{ max-width: 600px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        .error {{ color: #e74c3c; font-size: 24px; margin-bottom: 20px; }}
        .info {{ color: #2c3e50; margin: 10px 0; }}
        .description {{ background: #ffeaa7; padding: 15px; border-radius: 4px; margin: 15px 0; }}
    </style>
</head>
<body>
    <div class="container">
        <h1 class="error">✗ OAuth Authorization Failed</h1>
        <div class="info"><strong>Provider:</strong> {}</div>
        <div class="info"><strong>Error:</strong> {}</div>
        {}
        <p>Please try again or contact support if the problem persists.</p>
    </div>
</body>
</html>
"#,
            provider,
            provider,
            error,
            description
                .map(|d| format!("<div class=\"description\"><strong>Description:</strong> {}</div>", d))
                .unwrap_or_default()
        );

        Ok(template)
    }
}

/// OAuth notification handler for async notification processing
pub struct OAuthNotificationHandler {
    resources: Arc<ServerResources>,
}

impl OAuthNotificationHandler {
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    /// Handle OAuth completion notification
    pub async fn handle_notification(
        &self,
        notification: &crate::mcp::schema::OAuthCompletedNotification,
    ) -> Result<()> {
        info!(
            "Processing OAuth notification for user {} with provider {}",
            notification.user_id, notification.provider
        );

        // Process the notification (e.g., update user status, send confirmations, etc.)
        // This could involve database updates, external API calls, etc.

        Ok(())
    }

    /// Spawn a background task to handle OAuth notifications
    pub fn spawn_notification_handler(
        resources: Arc<ServerResources>,
        mut notification_receiver: tokio::sync::broadcast::Receiver<
            crate::mcp::schema::OAuthCompletedNotification,
        >,
    ) {
        let handler = Self::new(resources);

        tokio::spawn(async move {
            info!("OAuth notification handler started");

            while let Ok(notification) = notification_receiver.recv().await {
                if let Err(e) = handler.handle_notification(&notification).await {
                    error!("Failed to handle OAuth notification: {}", e);
                }
            }

            warn!("OAuth notification handler ended");
        });
    }
}

// Helper struct for API errors
struct ApiError(serde_json::Value);

impl warp::reject::Reject for ApiError {}