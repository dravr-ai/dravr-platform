// ABOUTME: PostgreSQL tenant, tool selection, LLM credential, and fitness config repositories
// ABOUTME: Manages multi-tenant isolation, tool catalog, LLM provider credentials, and fitness settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::{
    FitnessConfigRepository, LlmCredentialRepository, TenantRepository, ToolSelectionRepository,
};
use super::PostgresDatabase;
use crate::plugins::shared::encryption::HasEncryption;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::config::FitnessConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantOAuthCredentials;
use pierre_core::models::{LlmCredentialRecord, LlmCredentialSummary, TenantId};
use pierre_core::models::{
    OAuthApp, Tenant, TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory,
};
use sqlx::Row;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

#[async_trait]
impl TenantRepository for PostgresDatabase {
    // ================================
    // Multi-Tenant Management
    // ================================

    /// Create a new tenant
    async fn create(&self, tenant: &Tenant) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO tenants (id, name, slug, domain, subscription_tier, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, true, $6, $7)
            ",
        )
        .bind(tenant.id)
        .bind(&tenant.name)
        .bind(&tenant.slug)
        .bind(&tenant.domain)
        .bind(&tenant.plan)
        .bind(tenant.created_at)
        .bind(tenant.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create tenant: {e}")))?;

        // Add the owner as an admin of the tenant
        let tenant_user_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        sqlx::query(
            r"
            INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at)
            VALUES ($1, $2, $3, 'owner', $4, $4)
            ",
        )
        .bind(tenant_user_id)
        .bind(tenant.id)
        .bind(tenant.owner_user_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to add owner to tenant: {e}")))?;

        info!(
            "Created tenant: {} ({}) and added owner to tenant_users",
            tenant.name, tenant.id
        );
        Ok(())
    }

    /// Get tenant by ID
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant> {
        let row = sqlx::query_as::<_, (TenantId, String, String, Option<String>, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
            r"
            SELECT t.id, t.name, t.slug, t.domain, t.subscription_tier, tu.user_id, t.created_at, t.updated_at
            FROM tenants t
            JOIN tenant_users tu ON t.id = tu.tenant_id AND tu.role = 'owner'
            WHERE t.id = $1 AND t.is_active = true
            ",
        )
        .bind(tenant_id.0)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        match row {
            Some((id, name, slug, domain, plan, owner_user_id, created_at, updated_at)) => {
                Ok(Tenant {
                    id,
                    name,
                    slug,
                    domain,
                    plan,
                    owner_user_id,
                    created_at,
                    updated_at,
                })
            }
            None => Err(AppError::not_found(format!("Tenant {tenant_id}"))),
        }
    }

    /// Get tenant by slug
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant> {
        let row = sqlx::query_as::<_, (TenantId, String, String, Option<String>, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
            r"
            SELECT t.id, t.name, t.slug, t.domain, t.subscription_tier, tu.user_id, t.created_at, t.updated_at
            FROM tenants t
            JOIN tenant_users tu ON t.id = tu.tenant_id AND tu.role = 'owner'
            WHERE t.slug = $1 AND t.is_active = true
            ",
        )
        .bind(slug)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        match row {
            Some((id, name, slug, domain, plan, owner_user_id, created_at, updated_at)) => {
                Ok(Tenant {
                    id,
                    name,
                    slug,
                    domain,
                    plan,
                    owner_user_id,
                    created_at,
                    updated_at,
                })
            }
            None => Err(AppError::not_found(format!("Tenant {slug}"))),
        }
    }

    /// List tenants for a user
    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>> {
        let rows = sqlx::query_as::<
            _,
            (
                TenantId,
                String,
                String,
                Option<String>,
                String,
                Uuid,
                DateTime<Utc>,
                DateTime<Utc>,
                // tu.joined_at is included in SELECT so PostgreSQL allows
                // ORDER BY with DISTINCT (not mapped to Tenant)
                DateTime<Utc>,
            ),
        >(
            r"
            SELECT DISTINCT t.id, t.name, t.slug, t.domain, t.subscription_tier,
                   owner.user_id, t.created_at, t.updated_at, tu.joined_at
            FROM tenants t
            JOIN tenant_users tu ON t.id = tu.tenant_id
            JOIN tenant_users owner ON t.id = owner.tenant_id AND owner.role = 'owner'
            WHERE tu.user_id = $1 AND t.is_active = true
            ORDER BY tu.joined_at ASC
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let tenants = rows
            .into_iter()
            .map(
                |(
                    id,
                    name,
                    slug,
                    domain,
                    plan,
                    owner_user_id,
                    created_at,
                    updated_at,
                    _joined_at,
                )| Tenant {
                    id,
                    name,
                    slug,
                    domain,
                    plan,
                    owner_user_id,
                    created_at,
                    updated_at,
                },
            )
            .collect();

        Ok(tenants)
    }

    /// Store tenant OAuth credentials
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()> {
        // Encrypt the client secret using AES-256-GCM with AAD binding
        // AAD context format: "{tenant_id}|{provider}|tenant_oauth_credentials"
        let aad_context = format!(
            "{}|{}|tenant_oauth_credentials",
            credentials.tenant_id, credentials.provider
        );
        let encrypted_secret =
            HasEncryption::encrypt_data_with_aad(self, &credentials.client_secret, &aad_context)?;

        // Convert scopes Vec<String> to PostgreSQL array format
        let scopes_array: Vec<&str> = credentials.scopes.iter().map(String::as_str).collect();
        let now = chrono::Utc::now();

        sqlx::query(
            r"
            INSERT INTO tenant_oauth_credentials
                (tenant_id, provider, client_id, client_secret_encrypted,
                 redirect_uri, scopes, rate_limit_per_day, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, true, $8, $8)
            ON CONFLICT (tenant_id, provider)
            DO UPDATE SET
                client_id = EXCLUDED.client_id,
                client_secret_encrypted = EXCLUDED.client_secret_encrypted,
                redirect_uri = EXCLUDED.redirect_uri,
                scopes = EXCLUDED.scopes,
                rate_limit_per_day = EXCLUDED.rate_limit_per_day,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(credentials.tenant_id)
        .bind(&credentials.provider)
        .bind(&credentials.client_id)
        .bind(&encrypted_secret)
        .bind(&credentials.redirect_uri)
        .bind(&scopes_array)
        .bind(i32::try_from(credentials.rate_limit_per_day).unwrap_or(i32::MAX))
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store OAuth credentials: {e}")))?;

        Ok(())
    }

    /// Get tenant OAuth providers
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, Vec<String>, i32)>(
            r"
            SELECT provider, client_id, client_secret_encrypted,
                   redirect_uri, scopes, rate_limit_per_day
            FROM tenant_oauth_credentials
            WHERE tenant_id = $1 AND is_active = true
            ORDER BY provider
            ",
        )
        .bind(tenant_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let credentials = rows
            .into_iter()
            .map(
                |(provider, client_id, encrypted_secret, redirect_uri, scopes, rate_limit)| {
                    // Decrypt the client secret using AAD binding
                    // AAD context format: "{tenant_id}|{provider}|tenant_oauth_credentials"
                    let aad_context = format!("{tenant_id}|{provider}|tenant_oauth_credentials");
                    let client_secret = HasEncryption::decrypt_data_with_aad(
                        self,
                        &encrypted_secret,
                        &aad_context,
                    )?;

                    Ok(TenantOAuthCredentials {
                        tenant_id,
                        provider,
                        client_id,
                        client_secret,
                        redirect_uri,
                        scopes,
                        rate_limit_per_day: u32::try_from(rate_limit).unwrap_or(0),
                    })
                },
            )
            .collect::<AppResult<Vec<_>>>()?;

        Ok(credentials)
    }

    /// Get tenant OAuth credentials for specific provider
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>> {
        let row = sqlx::query_as::<_, (String, String, String, Vec<String>, i32)>(
            r"
            SELECT client_id, client_secret_encrypted,
                   redirect_uri, scopes, rate_limit_per_day
            FROM tenant_oauth_credentials
            WHERE tenant_id = $1 AND provider = $2 AND is_active = true
            ",
        )
        .bind(tenant_id.0)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        match row {
            Some((client_id, encrypted_secret, redirect_uri, scopes, rate_limit)) => {
                // Decrypt the client secret using AAD binding
                // AAD context format: "{tenant_id}|{provider}|tenant_oauth_credentials"
                let aad_context = format!("{tenant_id}|{provider}|tenant_oauth_credentials");
                let client_secret =
                    HasEncryption::decrypt_data_with_aad(self, &encrypted_secret, &aad_context)?;

                Ok(Some(TenantOAuthCredentials {
                    tenant_id,
                    provider: provider.to_owned(),
                    client_id,
                    client_secret,
                    redirect_uri,
                    scopes,
                    rate_limit_per_day: u32::try_from(rate_limit).unwrap_or(0),
                }))
            }
            None => Ok(None),
        }
    }

    // ================================
    // OAuth App Registration
    // ================================

    /// Create OAuth application
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()> {
        let redirect_uris: Vec<&str> = app.redirect_uris.iter().map(String::as_str).collect();
        let scopes: Vec<&str> = app.scopes.iter().map(String::as_str).collect();

        sqlx::query(
            r"
            INSERT INTO oauth_apps 
                (id, client_id, client_secret, name, description, redirect_uris, 
                 scopes, app_type, owner_user_id, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, $10, $11)
            ",
        )
        .bind(app.id)
        .bind(&app.client_id)
        .bind(&app.client_secret)
        .bind(&app.name)
        .bind(&app.description)
        .bind(&redirect_uris)
        .bind(&scopes)
        .bind(&app.app_type)
        .bind(app.owner_user_id)
        .bind(app.created_at)
        .bind(app.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create OAuth app: {e}")))?;

        Ok(())
    }

    /// Get OAuth app by client ID
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp> {
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                String,
                Option<String>,
                Vec<String>,
                Vec<String>,
                String,
                Uuid,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            r"
            SELECT id, client_id, client_secret, name, description, redirect_uris, 
                   scopes, app_type, owner_user_id, created_at, updated_at
            FROM oauth_apps
            WHERE client_id = $1 AND is_active = true
            ",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        match row {
            Some((
                id,
                client_id,
                client_secret,
                name,
                description,
                redirect_uris,
                scopes,
                app_type,
                owner_user_id,
                created_at,
                updated_at,
            )) => Ok(OAuthApp {
                id,
                client_id,
                client_secret,
                name,
                description,
                redirect_uris,
                scopes,
                app_type,
                owner_user_id,
                created_at,
                updated_at,
            }),
            None => Err(AppError::not_found(format!("OAuth app {client_id}"))),
        }
    }

    /// List OAuth apps for a user
    async fn list_oauth_apps_for_user(&self, user_id: Uuid) -> AppResult<Vec<OAuthApp>> {
        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                String,
                Option<String>,
                Vec<String>,
                Vec<String>,
                String,
                Uuid,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            r"
            SELECT id, client_id, client_secret, name, description, redirect_uris, 
                   scopes, app_type, owner_user_id, created_at, updated_at
            FROM oauth_apps
            WHERE owner_user_id = $1 AND is_active = true
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let apps = rows
            .into_iter()
            .map(
                |(
                    id,
                    client_id,
                    client_secret,
                    name,
                    description,
                    redirect_uris,
                    scopes,
                    app_type,
                    owner_user_id,
                    created_at,
                    updated_at,
                )| {
                    OAuthApp {
                        id,
                        client_id,
                        client_secret,
                        name,
                        description,
                        redirect_uris,
                        scopes,
                        app_type,
                        owner_user_id,
                        created_at,
                        updated_at,
                    }
                },
            )
            .collect();

        Ok(apps)
    }

    async fn get_all(&self) -> AppResult<Vec<Tenant>> {
        let query = r"
            SELECT id, slug, name, domain, plan, owner_user_id, created_at, updated_at
            FROM tenants
            WHERE is_active = true
            ORDER BY created_at
        ";

        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get all tenants: {e}")))?;

        let tenants = rows
            .iter()
            .map(|row| {
                use sqlx::Row;
                Ok(Tenant {
                    id: TenantId::from(
                        uuid::Uuid::parse_str(&row.try_get::<String, _>("id").map_err(|e| {
                            AppError::database(format!("Failed to parse id column: {e}"))
                        })?)
                        .map_err(|e| AppError::database(format!("Invalid tenant UUID: {e}")))?,
                    ),
                    name: row.try_get("name").map_err(|e| {
                        AppError::database(format!("Failed to parse name column: {e}"))
                    })?,
                    slug: row.try_get("slug").map_err(|e| {
                        AppError::database(format!("Failed to parse slug column: {e}"))
                    })?,
                    domain: row.try_get("domain").map_err(|e| {
                        AppError::database(format!("Failed to parse domain column: {e}"))
                    })?,
                    plan: row.try_get("plan").map_err(|e| {
                        AppError::database(format!("Failed to parse plan column: {e}"))
                    })?,
                    owner_user_id: uuid::Uuid::parse_str(
                        &row.try_get::<String, _>("owner_user_id").map_err(|e| {
                            AppError::database(format!("Failed to parse owner_user_id column: {e}"))
                        })?,
                    )
                    .map_err(|e| AppError::database(format!("Invalid user UUID: {e}")))?,
                    created_at: row.try_get("created_at").map_err(|e| {
                        AppError::database(format!("Failed to parse created_at column: {e}"))
                    })?,
                    updated_at: row.try_get("updated_at").map_err(|e| {
                        AppError::database(format!("Failed to parse updated_at column: {e}"))
                    })?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        Ok(tenants)
    }

    /// Get user role for a specific tenant
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT role FROM tenant_users WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(user_id)
        .bind(tenant_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        Ok(row.map(|r| r.0))
    }

    // ================================
    // Fitness Configuration Management
    // ================================

    // ================================
    // LLM Credentials Management
    // ================================

    // ================================
    // Tool Selection (PostgreSQL implementation)
    // ================================
}

#[async_trait]
impl ToolSelectionRepository for PostgresDatabase {
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>> {
        let rows = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            ORDER BY category, tool_name
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tool catalog: {e}")))?;

        rows.iter().map(Self::map_pg_tool_catalog_row).collect()
    }

    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>> {
        let row = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            WHERE tool_name = $1
            ",
        )
        .bind(tool_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tool catalog entry: {e}")))?;

        row.as_ref().map(Self::map_pg_tool_catalog_row).transpose()
    }

    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>> {
        let rows = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            WHERE category = $1
            ORDER BY tool_name
            ",
        )
        .bind(category.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tools by category: {e}")))?;

        rows.iter().map(Self::map_pg_tool_catalog_row).collect()
    }

    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>> {
        // Build list of acceptable plans based on hierarchy
        let acceptable_plans: Vec<&str> = match plan {
            TenantPlan::Starter => vec!["starter"],
            TenantPlan::Professional => vec!["starter", "professional"],
            TenantPlan::Enterprise => vec!["starter", "professional", "enterprise"],
        };

        let rows = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            WHERE min_plan = ANY($1)
            ORDER BY category, tool_name
            ",
        )
        .bind(&acceptable_plans)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tools by plan: {e}")))?;

        rows.iter().map(Self::map_pg_tool_catalog_row).collect()
    }

    async fn get_overrides(&self, tenant_id: TenantId) -> AppResult<Vec<TenantToolOverride>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, tool_name, is_enabled, enabled_by_user_id,
                   reason, created_at, updated_at
            FROM tenant_tool_overrides
            WHERE tenant_id = $1
            ORDER BY tool_name
            ",
        )
        .bind(tenant_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tenant tool overrides: {e}")))?;

        let overrides = rows
            .iter()
            .map(Self::map_pg_tenant_tool_override_row)
            .collect();
        Ok(overrides)
    }

    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, tool_name, is_enabled, enabled_by_user_id,
                   reason, created_at, updated_at
            FROM tenant_tool_overrides
            WHERE tenant_id = $1 AND tool_name = $2
            ",
        )
        .bind(tenant_id.0)
        .bind(tool_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tenant tool override: {e}")))?;

        Ok(row.as_ref().map(Self::map_pg_tenant_tool_override_row))
    }

    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride> {
        let now = Utc::now();
        let id = Uuid::new_v4();

        // Use PostgreSQL upsert syntax
        sqlx::query(
            r"
            INSERT INTO tenant_tool_overrides (id, tenant_id, tool_name, is_enabled, enabled_by_user_id, reason, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
            ON CONFLICT(tenant_id, tool_name) DO UPDATE SET
                is_enabled = EXCLUDED.is_enabled,
                enabled_by_user_id = EXCLUDED.enabled_by_user_id,
                reason = EXCLUDED.reason,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(id)
        .bind(tenant_id.0)
        .bind(tool_name)
        .bind(is_enabled)
        .bind(enabled_by_user_id)
        .bind(&reason)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tenant tool override: {e}")))?;

        // Fetch the resulting row (either inserted or updated)
        self.get_override(tenant_id, tool_name)
            .await?
            .ok_or_else(|| AppError::internal("Failed to retrieve upserted tenant tool override"))
    }

    async fn delete_override(&self, tenant_id: TenantId, tool_name: &str) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM tenant_tool_overrides
            WHERE tenant_id = $1 AND tool_name = $2
            ",
        )
        .bind(tenant_id.0)
        .bind(tool_name)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete tenant tool override: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize> {
        // Get tenant's plan to filter by plan restrictions
        let tenant = TenantRepository::get_by_id(self, tenant_id).await?;
        let plan = TenantPlan::parse_str(&tenant.plan)
            .ok_or_else(|| AppError::internal(format!("Invalid tenant plan: {}", tenant.plan)))?;

        // Get tools available for this plan
        let catalog = self.get_tools_by_min_plan(plan).await?;
        let overrides = self.get_overrides(tenant_id).await?;

        // Build override map
        let override_map: HashMap<String, bool> = overrides
            .into_iter()
            .map(|o| (o.tool_name, o.is_enabled))
            .collect();

        // Count enabled tools
        let count = catalog
            .iter()
            .filter(|tool| {
                override_map
                    .get(&tool.tool_name)
                    .copied()
                    .unwrap_or(tool.is_enabled_by_default)
            })
            .count();

        Ok(count)
    }

    async fn upsert_tool_catalog_entry(&self, entry: &ToolCatalogEntry) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO tool_catalog (id, tool_name, display_name, description, category,
                                      is_enabled_by_default, requires_provider, min_plan,
                                      created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NOW())
            ON CONFLICT(tool_name) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                category = EXCLUDED.category,
                is_enabled_by_default = EXCLUDED.is_enabled_by_default,
                requires_provider = EXCLUDED.requires_provider,
                min_plan = EXCLUDED.min_plan,
                updated_at = NOW()
            ",
        )
        .bind(&entry.id)
        .bind(&entry.tool_name)
        .bind(&entry.display_name)
        .bind(&entry.description)
        .bind(entry.category.as_str())
        .bind(entry.is_enabled_by_default)
        .bind(&entry.requires_provider)
        .bind(entry.min_plan.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tool catalog entry: {e}")))?;

        Ok(())
    }

    async fn delete_tool_catalog_entry(&self, tool_name: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM tool_catalog WHERE tool_name = $1")
            .bind(tool_name)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete tool catalog entry: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}

#[async_trait]
impl LlmCredentialRepository for PostgresDatabase {
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO user_llm_credentials (
                id, tenant_id, user_id, provider, api_key_encrypted,
                base_url, default_model, is_active, created_at, updated_at, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT(tenant_id, user_id, provider) DO UPDATE SET
                api_key_encrypted = EXCLUDED.api_key_encrypted,
                base_url = EXCLUDED.base_url,
                default_model = EXCLUDED.default_model,
                is_active = EXCLUDED.is_active,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(record.id)
        .bind(record.tenant_id)
        .bind(record.user_id)
        .bind(&record.provider)
        .bind(&record.api_key_encrypted)
        .bind(&record.base_url)
        .bind(&record.default_model)
        .bind(record.is_active)
        .bind(&record.created_at)
        .bind(&record.updated_at)
        .bind(record.created_by)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store LLM credentials: {e}")))?;

        Ok(())
    }

    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>> {
        let row = if let Some(uid) = user_id {
            sqlx::query(
                r"
                SELECT id, tenant_id, user_id, provider, api_key_encrypted,
                       base_url, default_model, is_active, created_at, updated_at, created_by
                FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND is_active = TRUE
                ",
            )
            .bind(tenant_id.to_string())
            .bind(uid)
            .bind(provider)
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT id, tenant_id, user_id, provider, api_key_encrypted,
                       base_url, default_model, is_active, created_at, updated_at, created_by
                FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id IS NULL AND provider = $2 AND is_active = TRUE
                ",
            )
            .bind(tenant_id.to_string())
            .bind(provider)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get LLM credentials: {e}")))?;

        row.map(|r| {
            let id_str: String = r.get("id");
            let tid_str: String = r.get("tenant_id");
            let created: DateTime<Utc> = r.get("created_at");
            let updated: DateTime<Utc> = r.get("updated_at");
            Ok(LlmCredentialRecord {
                id: Uuid::parse_str(&id_str)
                    .map_err(|e| AppError::internal(format!("Invalid credential UUID: {e}")))?,
                tenant_id: TenantId::from(
                    Uuid::parse_str(&tid_str)
                        .map_err(|e| AppError::internal(format!("Invalid tenant UUID: {e}")))?,
                ),
                user_id: r.get("user_id"),
                provider: r.get("provider"),
                api_key_encrypted: r.get("api_key_encrypted"),
                base_url: r.get("base_url"),
                default_model: r.get("default_model"),
                is_active: r.get("is_active"),
                created_at: created.to_rfc3339(),
                updated_at: updated.to_rfc3339(),
                created_by: r.get("created_by"),
            })
        })
        .transpose()
    }

    async fn list_credentials(&self, tenant_id: TenantId) -> AppResult<Vec<LlmCredentialSummary>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, base_url, default_model, is_active, created_at, updated_at
            FROM user_llm_credentials
            WHERE tenant_id = $1
            ORDER BY provider, user_id
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list LLM credentials: {e}")))?;

        rows.into_iter()
            .map(|r| {
                let id_str: String = r.get("id");
                let user_id: Option<Uuid> = r.get("user_id");
                let created: DateTime<Utc> = r.get("created_at");
                let updated: DateTime<Utc> = r.get("updated_at");
                Ok(LlmCredentialSummary {
                    id: Uuid::parse_str(&id_str)
                        .map_err(|e| AppError::internal(format!("Invalid credential UUID: {e}")))?,
                    user_id,
                    provider: r.get("provider"),
                    scope: if user_id.is_some() {
                        "user".to_owned()
                    } else {
                        "tenant".to_owned()
                    },
                    base_url: r.get("base_url"),
                    default_model: r.get("default_model"),
                    is_active: r.get("is_active"),
                    created_at: created.to_rfc3339(),
                    updated_at: updated.to_rfc3339(),
                })
            })
            .collect()
    }

    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<bool> {
        let result = if let Some(uid) = user_id {
            sqlx::query(
                r"
                DELETE FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id = $2 AND provider = $3
                ",
            )
            .bind(tenant_id.to_string())
            .bind(uid)
            .bind(provider)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                DELETE FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id IS NULL AND provider = $2
                ",
            )
            .bind(tenant_id.to_string())
            .bind(provider)
            .execute(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete LLM credentials: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<String>> {
        // Extract category from key (e.g., "llm.gemini_api_key" -> "llm_provider")
        let category = if config_key.starts_with("llm.") {
            "llm_provider"
        } else {
            config_key.split('.').next().unwrap_or("unknown")
        };

        let row = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT config_value
                FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id = $3
                ",
            )
            .bind(category)
            .bind(config_key)
            .bind(tid.to_string())
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT config_value
                FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id IS NULL
                ",
            )
            .bind(category)
            .bind(config_key)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get admin config override: {e}")))?;

        Ok(row.map(|r| r.get::<String, _>("config_value")))
    }
}

#[async_trait]
impl FitnessConfigRepository for PostgresDatabase {
    /// Save tenant-level fitness configuration
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        let config_json = serde_json::to_string(config)?;
        let now = chrono::Utc::now();

        let result = sqlx::query(
            r"
            INSERT INTO fitness_configurations (tenant_id, user_id, configuration_name, config_data, created_at, updated_at)
            VALUES ($1, NULL, $2, $3, $4, $4)
            ON CONFLICT (tenant_id, user_id, configuration_name)
            DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ",
        )
        .bind(tenant_id.to_string())
        .bind(configuration_name)
        .bind(&config_json)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch record: {e}")))?;

        Ok(result.get("id"))
    }

    /// Save user-specific fitness configuration
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        let config_json = serde_json::to_string(config)?;
        let now = chrono::Utc::now();

        let result = sqlx::query(
            r"
            INSERT INTO fitness_configurations (tenant_id, user_id, configuration_name, config_data, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $5)
            ON CONFLICT (tenant_id, user_id, configuration_name)
            DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(configuration_name)
        .bind(&config_json)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch record: {e}")))?;

        Ok(result.get("id"))
    }

    /// Get tenant-level fitness configuration
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        let result = sqlx::query(
            r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(configuration_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = result {
            let config_json: String = row.get("config_data");
            let config: FitnessConfig = serde_json::from_str(&config_json)?;
            Ok(Some(config))
        } else {
            Ok(None)
        }
    }

    /// Get user-specific fitness configuration
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        // First try to get user-specific configuration
        let result = sqlx::query(
            r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2 AND configuration_name = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(configuration_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = result {
            let config_json: String = row.get("config_data");
            let config: FitnessConfig = serde_json::from_str(&config_json)?;
            return Ok(Some(config));
        }

        // Fall back to tenant default configuration
        let result = sqlx::query(
            r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(configuration_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = result {
            let config_json: String = row.get("config_data");
            let config: FitnessConfig = serde_json::from_str(&config_json)?;
            Ok(Some(config))
        } else {
            Ok(None)
        }
    }

    /// List all tenant-level fitness configuration names
    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT configuration_name FROM fitness_configurations
            WHERE tenant_id = $1
            ORDER BY configuration_name
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let configurations = rows
            .into_iter()
            .map(|row| row.get::<String, _>("configuration_name"))
            .collect();

        Ok(configurations)
    }

    /// List all user-specific fitness configuration names
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT configuration_name FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY configuration_name
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let configurations = rows
            .into_iter()
            .map(|row| row.get::<String, _>("configuration_name"))
            .collect();

        Ok(configurations)
    }

    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool> {
        let rows_affected = if let Some(uid) = user_id {
            sqlx::query(
                r"
                DELETE FROM fitness_configurations
                WHERE tenant_id = $1 AND user_id = $2 AND configuration_name = $3
                ",
            )
            .bind(tenant_id.to_string())
            .bind(uid)
            .bind(configuration_name)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?
        } else {
            sqlx::query(
                r"
                DELETE FROM fitness_configurations
                WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
                ",
            )
            .bind(tenant_id.to_string())
            .bind(configuration_name)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?
        };

        Ok(rows_affected.rows_affected() > 0)
    }
}
