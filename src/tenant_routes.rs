// ABOUTME: HTTP REST API routes for multi-tenant management
// ABOUTME: Handles tenant creation and listing with tenant-isolated authentication flows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::{
    auth::AuthResult,
    database_plugins::{factory::Database, TenantDbOps, UserDbOps},
    errors::{AppError, AppResult},
    models::{Tenant, TenantId},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

// Tenant Management Request/Response Types

/// Request body for creating a new tenant
#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    /// Display name for the tenant
    pub name: String,
    /// URL-safe slug identifier for the tenant
    pub slug: String,
    /// Optional custom domain for the tenant
    pub domain: Option<String>,
    /// Subscription plan (basic, pro, enterprise)
    pub plan: Option<String>,
}

/// Response containing created tenant details
#[derive(Debug, Serialize)]
pub struct CreateTenantResponse {
    /// UUID of the created tenant
    pub tenant_id: String,
    /// Display name of the tenant
    pub name: String,
    /// URL-safe slug identifier
    pub slug: String,
    /// Custom domain if configured
    pub domain: Option<String>,
    /// ISO 8601 timestamp of creation
    pub created_at: String,
    /// API endpoint URL for this tenant
    pub api_endpoint: String,
}

/// Response containing list of tenants with pagination
#[derive(Debug, Serialize)]
pub struct TenantListResponse {
    /// List of tenant summaries
    pub tenants: Vec<TenantSummary>,
    /// Total number of tenants
    pub total_count: usize,
}

/// Summary information about a tenant
#[derive(Debug, Serialize)]
pub struct TenantSummary {
    /// UUID of the tenant
    pub tenant_id: String,
    /// Display name
    pub name: String,
    /// URL-safe slug
    pub slug: String,
    /// Custom domain if any
    pub domain: Option<String>,
    /// Subscription plan
    pub plan: String,
    /// ISO 8601 creation timestamp
    pub created_at: String,
    /// List of configured OAuth providers
    pub oauth_providers: Vec<String>,
}

// Route Handler Implementations

/// Create a new tenant organization
///
/// # Errors
///
/// Returns an error if:
/// - Tenant slug already exists
/// - Database operations fail
/// - User lacks permissions
pub async fn create_tenant(
    tenant_request: CreateTenantRequest,
    auth_result: AuthResult,
    database: Arc<Database>,
) -> AppResult<CreateTenantResponse> {
    info!("Creating new tenant: {}", tenant_request.name);

    // SECURITY: Global lookup — creating a new tenant, no tenant context yet
    database
        .get_user_global(auth_result.user_id)
        .await
        .map_err(|e| AppError::database(e.to_string()))?;

    // Generate tenant ID and validate slug uniqueness
    let tenant_id = TenantId::new();
    let slug = tenant_request.slug.trim().to_lowercase();

    // Check if slug already exists
    if let Ok(_existing) = database.get_tenant_by_slug(&slug).await {
        return Err(AppError::invalid_input(format!(
            "Tenant slug '{slug}' already exists"
        )));
    }

    // Create tenant in database
    let tenant_data = Tenant {
        id: tenant_id,
        name: tenant_request.name.clone(), // Safe: String ownership for tenant struct
        slug: slug.clone(),                // Safe: String ownership for tenant struct
        domain: tenant_request.domain.clone(), // Safe: String ownership for tenant struct
        plan: tenant_request.plan.unwrap_or_else(|| "basic".to_owned()),
        owner_user_id: auth_result.user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    database
        .create_tenant(&tenant_data)
        .await
        .map_err(|e| AppError::database(e.to_string()))?;

    info!(
        "Tenant created successfully: {} ({})",
        tenant_data.name, tenant_data.id
    );

    Ok(CreateTenantResponse {
        tenant_id: tenant_data.id.to_string(),
        name: tenant_data.name,
        slug: tenant_data.slug,
        domain: tenant_data.domain,
        created_at: tenant_data.created_at.to_rfc3339(),
        api_endpoint: format!("https://api.your-server.com/tenants/{}", tenant_data.id),
    })
}

/// List all tenants for the authenticated user
///
/// # Errors
///
/// Returns an error if:
/// - Database operations fail
/// - User lacks permissions
pub async fn list_tenants(
    auth_result: AuthResult,
    database: Arc<Database>,
) -> AppResult<TenantListResponse> {
    info!("Listing tenants for user: {}", auth_result.user_id);

    let tenants = database
        .list_tenants_for_user(auth_result.user_id)
        .await
        .map_err(|e| AppError::database(e.to_string()))?;

    let mut tenant_summaries = Vec::new();

    for tenant in tenants {
        // Get OAuth providers for this tenant
        let oauth_providers = database
            .get_tenant_oauth_providers(tenant.id)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    tenant_id = %tenant.id,
                    tenant_name = %tenant.name,
                    error = %e,
                    "Failed to fetch OAuth providers for tenant summary, using empty list"
                );
                Vec::new()
            });

        tenant_summaries.push(TenantSummary {
            tenant_id: tenant.id.to_string(),
            name: tenant.name,
            slug: tenant.slug,
            domain: tenant.domain,
            plan: tenant.plan,
            created_at: tenant.created_at.to_rfc3339(),
            oauth_providers: oauth_providers.into_iter().map(|p| p.provider).collect(),
        });
    }

    Ok(TenantListResponse {
        total_count: tenant_summaries.len(),
        tenants: tenant_summaries,
    })
}
