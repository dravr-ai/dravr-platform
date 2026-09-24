// ABOUTME: Repository trait, statements and shared body for tenants, their OAuth credentials and OAuth apps
// ABOUTME: One SQL text per operation; each backend shell supplies its uuid codec and its list-column codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tenants, written once.
//!
//! A tenant row, its owner through `tenant_users`, the per-provider OAuth
//! client credentials it registered (client secret enveloped under
//! AES-256-GCM, bound by AAD to tenant and provider), and the OAuth apps a
//! user registered for MCP clients. Two things differ per backend and come
//! in as macro arguments: the uuid columns (`tenant_users.user_id`,
//! `oauth_apps.id`, `oauth_apps.owner_user_id`), native on Postgres and
//! `TEXT` on `SQLite`, through the [`uuid_columns`](super::uuid_columns)
//! codec; and the list columns (`scopes`, `redirect_uris`), `TEXT[]` on
//! Postgres and a JSON array in `TEXT` on `SQLite`, through the
//! [`list_columns`](super::list_columns) codec. A [`TenantId`] binds and
//! reads natively on both drivers, so it needs neither.
//!
//! `$n` placeholders throughout; timestamps bind as [`DateTime<Utc>`] on
//! both; `is_active` binds and reads as `bool`. `list_for_user` selects
//! `tu.joined_at` beside the tenant columns because Postgres requires an
//! `ORDER BY` expression to appear in a `SELECT DISTINCT` list.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use pierre_core::models::{OAuthApp, Tenant};
use pierre_core::models::{TenantId, TenantOAuthCredentials};
use uuid::Uuid;

/// Multi-tenant management repository
#[async_trait]
pub trait TenantRepository: Send + Sync {
    /// Create a new tenant
    async fn create(&self, tenant: &Tenant) -> AppResult<()>;
    /// Get tenant by ID
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant>;
    /// Get tenant by slug
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant>;
    /// List tenants for a user
    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>>;
    /// Every tenant the user holds a membership row in, whatever the tenant's
    /// state or ownership, ordered by id.
    ///
    /// [`Self::list_for_user`] names only active tenants that have an owner,
    /// which is what a session may act in. This is how far a user's rows
    /// reach, which is what an operator acting on the whole user must be
    /// allowed to touch.
    async fn list_membership_tenant_ids(&self, user_id: Uuid) -> AppResult<Vec<TenantId>>;

    /// The agent this user has selected within this tenant, if any.
    ///
    /// The single answer to "which coach is this user's?". It replaced three
    /// disagreeing ones — `users.default_coach_id`, `agent_assignments.is_active`
    /// and per-conversation overrides — where the surface that wrote and the
    /// surface that read were often different, so a user could finish onboarding
    /// on one and read as un-onboarded on another.
    ///
    /// Scoped per membership because agents are tenant-scoped: a user in two
    /// tenants selects independently in each.
    async fn get_selected_agent(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<String>>;

    /// Set (or clear, with `None`) this user's selected agent in this tenant.
    ///
    /// "At most one" is structural — one column on a row that `UNIQUE(tenant_id,
    /// user_id)` already makes unique — rather than maintained by clearing every
    /// row and setting one, which could leave zero or two.
    async fn set_selected_agent(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        agent_id: Option<&str>,
    ) -> AppResult<()>;
    /// Store tenant OAuth credentials
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()>;
    /// Get tenant OAuth providers
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>>;
    /// Get tenant OAuth credentials for specific provider
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>>;
    /// Create OAuth application for MCP clients
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()>;
    /// Get OAuth app by client ID
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp>;
    /// Get all tenants for key rotation check
    async fn get_all(&self) -> AppResult<Vec<Tenant>>;
    /// Get user role for a specific tenant
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>>;
    /// Set the tenant's billing plan (Starter / Professional / Enterprise).
    ///
    /// Called by Stripe webhook handlers when a subscription state change
    /// implies a plan-tier flip on the tenant. Owner-driven plan changes
    /// always cascade through this method so audit logging stays uniform.
    async fn set_plan(&self, tenant_id: TenantId, plan: &str) -> AppResult<Tenant>;
}

/// Create the tenant row; the owner's membership is a second statement.
pub(crate) const CREATE_TENANT_SQL: &str = r"
            INSERT INTO tenants (id, name, slug, domain, subscription_tier, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, true, $6, $7)
            ";

/// Record the owner as the tenant's first member.
pub(crate) const ADD_TENANT_OWNER_SQL: &str = r"
            INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at)
            VALUES ($1, $2, $3, 'owner', $4, $4)
            ";

/// The columns [`tenant_from_row`] reads, with the owner joined in.
macro_rules! tenant_columns {
    () => {
        "t.id, t.name, t.slug, t.domain, t.subscription_tier, \
         tu.user_id AS owner_user_id, t.created_at, t.updated_at"
    };
}

/// One active tenant by id.
pub(crate) const GET_TENANT_BY_ID_SQL: &str = concat!(
    "SELECT ",
    tenant_columns!(),
    " FROM tenants t
            JOIN tenant_users tu ON t.id = tu.tenant_id AND tu.role = 'owner'
            WHERE t.id = $1 AND t.is_active = true"
);

/// One active tenant by slug.
pub(crate) const GET_TENANT_BY_SLUG_SQL: &str = concat!(
    "SELECT ",
    tenant_columns!(),
    " FROM tenants t
            JOIN tenant_users tu ON t.id = tu.tenant_id AND tu.role = 'owner'
            WHERE t.slug = $1 AND t.is_active = true"
);

/// Every active tenant the user is a member of, in the order they joined.
pub(crate) const LIST_TENANTS_FOR_USER_SQL: &str = r"
            SELECT DISTINCT t.id, t.name, t.slug, t.domain, t.subscription_tier,
                   owner.user_id AS owner_user_id, t.created_at, t.updated_at, tu.joined_at
            FROM tenants t
            JOIN tenant_users tu ON t.id = tu.tenant_id
            JOIN tenant_users owner ON t.id = owner.tenant_id AND owner.role = 'owner'
            WHERE tu.user_id = $1 AND t.is_active = true
            ORDER BY tu.joined_at ASC
            ";

/// Every tenant a user holds a membership row in, active or not, owned or not.
pub(crate) const LIST_MEMBERSHIP_TENANT_IDS_SQL: &str =
    "SELECT tenant_id FROM tenant_users WHERE user_id = $1 ORDER BY tenant_id";

/// Every active tenant, oldest first.
pub(crate) const GET_ALL_TENANTS_SQL: &str = concat!(
    "SELECT ",
    tenant_columns!(),
    " FROM tenants t
            JOIN tenant_users tu ON tu.tenant_id = t.id AND tu.role = 'owner'
            WHERE t.is_active = true
            ORDER BY t.created_at"
);

/// Store or replace the tenant's client credentials for one provider.
pub(crate) const STORE_TENANT_OAUTH_CREDENTIALS_SQL: &str = r"
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
            ";

/// Every provider the tenant has live credentials for.
pub(crate) const GET_TENANT_OAUTH_PROVIDERS_SQL: &str = r"
            SELECT provider, client_id, client_secret_encrypted,
                   redirect_uri, scopes, rate_limit_per_day
            FROM tenant_oauth_credentials
            WHERE tenant_id = $1 AND is_active = true
            ORDER BY provider
            ";

/// The tenant's live credentials for one provider.
pub(crate) const GET_TENANT_OAUTH_CREDENTIALS_SQL: &str = r"
            SELECT provider, client_id, client_secret_encrypted,
                   redirect_uri, scopes, rate_limit_per_day
            FROM tenant_oauth_credentials
            WHERE tenant_id = $1 AND provider = $2 AND is_active = true
            ";

/// Register an OAuth app.
pub(crate) const CREATE_OAUTH_APP_SQL: &str = r"
            INSERT INTO oauth_apps
                (id, client_id, client_secret, name, description, redirect_uris,
                 scopes, app_type, owner_user_id, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, $10, $11)
            ";

/// One live OAuth app by its client id.
pub(crate) const GET_OAUTH_APP_BY_CLIENT_ID_SQL: &str = r"
            SELECT id, client_id, client_secret, name, description, redirect_uris,
                   scopes, app_type, owner_user_id, created_at, updated_at
            FROM oauth_apps
            WHERE client_id = $1 AND is_active = true
            ";

/// The agent a member has selected in a tenant.
pub(crate) const GET_SELECTED_AGENT_SQL: &str =
    "SELECT selected_agent_id FROM tenant_users WHERE tenant_id = $1 AND user_id = $2";

/// Select, or clear, a member's agent in a tenant.
pub(crate) const SET_SELECTED_AGENT_SQL: &str =
    "UPDATE tenant_users SET selected_agent_id = $3 WHERE tenant_id = $1 AND user_id = $2";

/// A member's role in a tenant.
pub(crate) const GET_USER_TENANT_ROLE_SQL: &str =
    "SELECT role FROM tenant_users WHERE user_id = $1 AND tenant_id = $2";

/// Move the tenant to a plan.
pub(crate) const SET_TENANT_PLAN_SQL: &str =
    "UPDATE tenants SET subscription_tier = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2";

/// The AAD a tenant's client secret is bound to: tenant, provider and the
/// table, so a ciphertext moved between rows fails to open.
pub(crate) fn tenant_oauth_aad(tenant_id: TenantId, provider: &str) -> String {
    format!("{tenant_id}|{provider}|tenant_oauth_credentials")
}

/// Decode one tenant row. `owner_user_id` is read by the caller through its
/// backend's codec; every other column decodes the same way on both drivers.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn tenant_from_row<R>(row: &R, owner_user_id: Uuid) -> AppResult<Tenant>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |col: &str, e: sqlx::Error| AppError::database(format!("Failed to get tenants.{col}: {e}"));
    Ok(Tenant {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        name: row.try_get("name").map_err(|e| column("name", e))?,
        slug: row.try_get("slug").map_err(|e| column("slug", e))?,
        domain: row.try_get("domain").map_err(|e| column("domain", e))?,
        plan: row
            .try_get("subscription_tier")
            .map_err(|e| column("subscription_tier", e))?,
        owner_user_id,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column("updated_at", e))?,
    })
}

/// Decode one credentials row, opening the client secret. `scopes` is read
/// by the caller through its backend's list codec.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or the decryption error when the secret does not open under its AAD.
pub(crate) fn tenant_oauth_credentials_from_row<R>(
    row: &R,
    tenant_id: TenantId,
    scopes: Vec<String>,
    decrypt: impl FnOnce(&str, &str) -> AppResult<String>,
) -> AppResult<TenantOAuthCredentials>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get tenant_oauth_credentials.{col}: {e}"))
    };
    let provider: String = row.try_get("provider").map_err(|e| column("provider", e))?;
    let encrypted_secret: String = row
        .try_get("client_secret_encrypted")
        .map_err(|e| column("client_secret_encrypted", e))?;
    let client_secret = decrypt(&encrypted_secret, &tenant_oauth_aad(tenant_id, &provider))?;
    let rate_limit: i32 = row
        .try_get("rate_limit_per_day")
        .map_err(|e| column("rate_limit_per_day", e))?;
    Ok(TenantOAuthCredentials {
        tenant_id,
        provider,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column("client_id", e))?,
        client_secret,
        redirect_uri: row
            .try_get("redirect_uri")
            .map_err(|e| column("redirect_uri", e))?,
        scopes,
        rate_limit_per_day: u32::try_from(rate_limit).unwrap_or(0),
    })
}

/// Decode one OAuth app row. The two uuid columns and the two list columns
/// are read by the caller through its backend's codecs.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn oauth_app_from_row<R>(
    row: &R,
    id: Uuid,
    owner_user_id: Uuid,
    redirect_uris: Vec<String>,
    scopes: Vec<String>,
) -> AppResult<OAuthApp>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get oauth_apps.{col}: {e}"))
    };
    Ok(OAuthApp {
        id,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column("client_id", e))?,
        client_secret: row
            .try_get("client_secret")
            .map_err(|e| column("client_secret", e))?,
        name: row.try_get("name").map_err(|e| column("name", e))?,
        description: row
            .try_get("description")
            .map_err(|e| column("description", e))?,
        redirect_uris,
        scopes,
        app_type: row.try_get("app_type").map_err(|e| column("app_type", e))?,
        owner_user_id,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column("updated_at", e))?,
    })
}

/// Emit the whole [`TenantRepository`] implementation for one backend type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec and
/// `$lists` its [`list_columns`](super::list_columns) codec. The body is
/// written once here; each backend's shell invokes it with its own type, and
/// sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_tenant_repository {
    ($ty:ty, $ids:ident, $lists:ident) => {
        #[async_trait::async_trait]
        impl TenantRepository for $ty {
            async fn get_selected_agent(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query(GET_SELECTED_AGENT_SQL)
                    .bind(tenant_id)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read selected coach: {e}"))
                    })?;

                row.map(|r| {
                    r.try_get::<Option<String>, _>("selected_agent_id")
                        .map_err(|e| {
                            AppError::database(format!("Failed to get selected_agent_id: {e}"))
                        })
                })
                .transpose()
                .map(Option::flatten)
            }

            async fn set_selected_agent(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                agent_id: Option<&str>,
            ) -> AppResult<()> {
                sqlx::query(SET_SELECTED_AGENT_SQL)
                    .bind(tenant_id)
                    .bind($ids::bind(user_id))
                    .bind(agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set selected coach: {e}"))
                    })?;

                Ok(())
            }

            async fn create(&self, tenant: &Tenant) -> AppResult<()> {
                sqlx::query(CREATE_TENANT_SQL)
                    .bind(tenant.id)
                    .bind(&tenant.name)
                    .bind(&tenant.slug)
                    .bind(&tenant.domain)
                    .bind(&tenant.plan)
                    .bind(tenant.created_at)
                    .bind(tenant.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create tenant: {e}")))?;

                sqlx::query(ADD_TENANT_OWNER_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(tenant.id)
                    .bind($ids::bind(tenant.owner_user_id))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to add owner to tenant: {e}"))
                    })?;

                info!(
                    "Created tenant: {} ({}) and added owner to tenant_users",
                    tenant.name, tenant.id
                );
                Ok(())
            }

            async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant> {
                let row = sqlx::query(GET_TENANT_BY_ID_SQL)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get tenant: {e}")))?;

                match row {
                    Some(row) => tenant_from_row(&row, $ids::read(&row, "owner_user_id")?),
                    None => Err(AppError::not_found(format!("Tenant {tenant_id}"))),
                }
            }

            async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant> {
                let row = sqlx::query(GET_TENANT_BY_SLUG_SQL)
                    .bind(slug)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get tenant: {e}")))?;

                match row {
                    Some(row) => tenant_from_row(&row, $ids::read(&row, "owner_user_id")?),
                    None => Err(AppError::not_found(format!("Tenant {slug}"))),
                }
            }

            async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>> {
                let rows = sqlx::query(LIST_TENANTS_FOR_USER_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list tenants: {e}")))?;

                rows.iter()
                    .map(|row| tenant_from_row(row, $ids::read(row, "owner_user_id")?))
                    .collect()
            }

            async fn list_membership_tenant_ids(&self, user_id: Uuid) -> AppResult<Vec<TenantId>> {
                let rows = sqlx::query(LIST_MEMBERSHIP_TENANT_IDS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list tenant memberships: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        row.try_get::<TenantId, _>("tenant_id").map_err(|e| {
                            AppError::database(format!("Failed to get tenant_users.tenant_id: {e}"))
                        })
                    })
                    .collect()
            }

            async fn store_oauth_credentials(
                &self,
                credentials: &TenantOAuthCredentials,
            ) -> AppResult<()> {
                let encrypted_secret = HasEncryption::encrypt_data_with_aad(
                    self,
                    &credentials.client_secret,
                    &tenant_oauth_aad(credentials.tenant_id, &credentials.provider),
                )?;

                sqlx::query(STORE_TENANT_OAUTH_CREDENTIALS_SQL)
                    .bind(credentials.tenant_id)
                    .bind(&credentials.provider)
                    .bind(&credentials.client_id)
                    .bind(&encrypted_secret)
                    .bind(&credentials.redirect_uri)
                    .bind($lists::bind_json(&credentials.scopes))
                    .bind(i32::try_from(credentials.rate_limit_per_day).unwrap_or(i32::MAX))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth credentials: {e}"))
                    })?;

                Ok(())
            }

            async fn get_oauth_providers(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Vec<TenantOAuthCredentials>> {
                let rows = sqlx::query(GET_TENANT_OAUTH_PROVIDERS_SQL)
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list OAuth providers: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        tenant_oauth_credentials_from_row(
                            row,
                            tenant_id,
                            $lists::read_json(row, "scopes")?,
                            |encrypted, aad| {
                                HasEncryption::decrypt_data_with_aad(self, encrypted, aad)
                            },
                        )
                    })
                    .collect()
            }

            async fn get_oauth_credentials(
                &self,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<TenantOAuthCredentials>> {
                let row = sqlx::query(GET_TENANT_OAUTH_CREDENTIALS_SQL)
                    .bind(tenant_id)
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get OAuth credentials: {e}"))
                    })?;

                row.map(|row| {
                    tenant_oauth_credentials_from_row(
                        &row,
                        tenant_id,
                        $lists::read_json(&row, "scopes")?,
                        |encrypted, aad| HasEncryption::decrypt_data_with_aad(self, encrypted, aad),
                    )
                })
                .transpose()
            }

            async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()> {
                sqlx::query(CREATE_OAUTH_APP_SQL)
                    .bind($ids::bind(app.id))
                    .bind(&app.client_id)
                    .bind(&app.client_secret)
                    .bind(&app.name)
                    .bind(&app.description)
                    .bind($lists::bind_json(&app.redirect_uris))
                    .bind($lists::bind_json(&app.scopes))
                    .bind(&app.app_type)
                    .bind($ids::bind(app.owner_user_id))
                    .bind(app.created_at)
                    .bind(app.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create OAuth app: {e}")))?;

                Ok(())
            }

            async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp> {
                let row = sqlx::query(GET_OAUTH_APP_BY_CLIENT_ID_SQL)
                    .bind(client_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get OAuth app: {e}")))?;

                match row {
                    Some(row) => oauth_app_from_row(
                        &row,
                        $ids::read(&row, "id")?,
                        $ids::read(&row, "owner_user_id")?,
                        $lists::read_json(&row, "redirect_uris")?,
                        $lists::read_json(&row, "scopes")?,
                    ),
                    None => Err(AppError::not_found(format!("OAuth app {client_id}"))),
                }
            }

            async fn get_all(&self) -> AppResult<Vec<Tenant>> {
                let rows = sqlx::query(GET_ALL_TENANTS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get all tenants: {e}")))?;

                rows.iter()
                    .map(|row| tenant_from_row(row, $ids::read(row, "owner_user_id")?))
                    .collect()
            }

            async fn get_user_role(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query(GET_USER_TENANT_ROLE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user role: {e}")))?;

                row.map(|r| {
                    r.try_get::<String, _>("role")
                        .map_err(|e| AppError::database(format!("Failed to get role: {e}")))
                })
                .transpose()
            }

            async fn set_plan(&self, tenant_id: TenantId, plan: &str) -> AppResult<Tenant> {
                let result = sqlx::query(SET_TENANT_PLAN_SQL)
                    .bind(plan)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to set tenant plan: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("Tenant {tenant_id}")));
                }

                self.get_by_id(tenant_id).await
            }
        }
    };
}
pub(crate) use impl_tenant_repository;
