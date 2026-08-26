// ABOUTME: Configuration service with caching and hot reload support
// ABOUTME: Provides runtime configuration access with database override resolution
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::manager::AdminConfigManager;
use super::repository::{AdminConfigRepository, LogChangeParams, SetOverrideParams};
use chrono::Utc;
use pierre_config::admin_definitions::{
    register_activity_access_quotas, register_algorithm_selection, register_cache_ttl,
    register_feature_flags, register_fitbit_provider, register_garmin_provider,
    register_group_permissions, register_heart_rate_zones, register_llm_pricing,
    register_llm_provider_config, register_mcp_network, register_monitoring, register_nutrition,
    register_rate_limiting, register_recommendation_engine, register_sleep_recovery,
    register_sqlx_pool, register_strava_provider, register_tokio_runtime, register_tool_execution,
    register_training_stress_balance, register_usage_quotas, register_weather_analysis,
    ParameterDefinition,
};
use pierre_config::admin_env::{EnvConfigError, EnvConfigPins};
use pierre_config::admin_types::{
    validate_parameter_value, AdminConfigCategory, AdminConfigParameter, ConfigAuditEntry,
    ConfigAuditFilter, ConfigCatalogResponse, ConfigOverride, ConfigScope, ConfigValidationError,
    ResetConfigRequest, ResetConfigResponse, UpdateConfigRequest, UpdateConfigResponse,
    ValidateConfigRequest, ValidateConfigResponse,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_runtime_context::ConfigLookupScope;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Admin configuration service for managing runtime configuration
pub struct AdminConfigService {
    manager: Box<dyn AdminConfigRepository>,
    /// Cached parameter definitions (loaded at startup)
    definitions: Arc<RwLock<HashMap<String, ParameterDefinition>>>,
    /// Environment pins captured once at construction. Layered under
    /// per-tenant and per-user overrides but above the system-wide row, so a
    /// deploy-time pin beats a runtime admin edit at the same scope.
    env_pins: EnvConfigPins,
    /// Category metadata
    categories: Arc<RwLock<Vec<AdminConfigCategory>>>,
}

impl AdminConfigService {
    /// Create an admin config service backed by `SQLite`
    ///
    /// # Errors
    ///
    /// Returns an error if the initial cache refresh fails.
    pub async fn new(pool: SqlitePool) -> AppResult<Self> {
        Self::from_repository(Box::new(AdminConfigManager::new(pool))).await
    }

    /// Create an admin config service backed by `PostgreSQL`
    ///
    /// # Errors
    ///
    /// Returns an error if the initial cache refresh fails.
    #[cfg(feature = "postgresql")]
    pub async fn from_postgres(pool: sqlx::PgPool) -> AppResult<Self> {
        use super::postgres_manager::PostgresAdminConfigManager;
        Self::from_repository(Box::new(PostgresAdminConfigManager::new(pool))).await
    }

    /// Create an admin config service from any repository implementation
    ///
    /// # Errors
    ///
    /// Returns an error if the initial cache refresh fails.
    async fn from_repository(manager: Box<dyn AdminConfigRepository>) -> AppResult<Self> {
        // Load categories from database
        let categories = manager.get_categories().await.unwrap_or_default();

        let definitions = Self::build_definitions();
        info!(
            "Initialized {} admin configuration parameter definitions",
            definitions.len()
        );

        // Capture environment pins once: the process environment does not
        // change under a running server, and re-reading it per lookup would
        // make config resolution depend on ambient state.
        let (env_pins, env_errors) = EnvConfigPins::capture(&definitions);
        if !env_errors.is_empty() {
            let detail = env_errors
                .iter()
                .map(EnvConfigError::describe)
                .collect::<Vec<_>>()
                .join("; ");
            // Fail the boot rather than degrade to "ignored". A pin that
            // silently does nothing is the exact failure this layer exists to
            // remove.
            return Err(AppError::invalid_input(format!(
                "Invalid config environment {}: {detail}",
                if env_errors.len() == 1 {
                    "variable"
                } else {
                    "variables"
                }
            )));
        }
        if !env_pins.is_empty() {
            info!(
                pinned = env_pins.len(),
                "Admin config parameters pinned by environment variables"
            );
        }

        let service = Self {
            manager,
            definitions: Arc::new(RwLock::new(definitions)),
            env_pins,
            categories: Arc::new(RwLock::new(categories)),
        };
        service.warn_on_shadowed_overrides().await?;
        Ok(service)
    }

    /// Warn at boot for every key an environment pin outranks a stored
    /// system-wide override on.
    ///
    /// The pin wins by design, but the operator who saved that override sees
    /// their value simply not apply. Announcing it once at startup is what
    /// turns "the config write did nothing" into a question with an answer.
    /// Bounded by the number of pins, so this is a handful of reads at most.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the stored overrides fails.
    async fn warn_on_shadowed_overrides(&self) -> AppResult<()> {
        if self.env_pins.is_empty() {
            return Ok(());
        }
        let definitions = self.definitions.read().await.clone();

        for def in definitions.values() {
            let Some(pinned) = self.env_pins.get(&def.key) else {
                continue;
            };
            let Some(stored) = self
                .manager
                .get_override(&def.category, &def.key, ConfigScope::Global)
                .await?
            else {
                continue;
            };
            warn!(
                key = %def.key,
                env_variable = def.env.as_ref().map_or("", |b| b.name.as_str()),
                pinned_value = %pinned,
                shadowed_value = %stored.config_value,
                "Environment pin outranks the stored system-wide override; \
                 the stored value will not apply until the variable is unset"
            );
        }
        Ok(())
    }

    /// Keys in `parameters` that an environment pin would outrank at
    /// `scope`. Only the system-wide scope is shadowed — a tenant or per-user
    /// row is narrower than the fleet and wins over a pin.
    fn shadowed_by_env(
        &self,
        parameters: &HashMap<String, serde_json::Value>,
        scope: ConfigScope<'_>,
    ) -> Vec<String> {
        if !matches!(scope, ConfigScope::Global) {
            return Vec::new();
        }
        let mut shadowed: Vec<String> = parameters
            .keys()
            .filter(|key| self.env_pins.get(key).is_some())
            .cloned()
            .collect();
        shadowed.sort();
        shadowed
    }

    /// Build the parameter catalog. Pure — the caller owns where it lands,
    /// so env pins can be validated against it before the service exists.
    fn build_definitions() -> HashMap<String, ParameterDefinition> {
        let mut defs = HashMap::new();

        // Rate Limiting Parameters — see config::admin::definitions::register_rate_limiting
        register_rate_limiting(&mut defs);

        // Feature Flags — see config::admin::definitions::register_feature_flags
        register_feature_flags(&mut defs);

        // LLM Provider Configuration — see config::admin::definitions::register_llm_provider_config
        register_llm_provider_config(&mut defs);

        // Heart Rate Zones — see config::admin::definitions::register_heart_rate_zones
        register_heart_rate_zones(&mut defs);

        // Algorithm Selection — see config::admin::definitions::register_algorithm_selection
        register_algorithm_selection(&mut defs);

        // Recommendation Engine — see config::admin::definitions::register_recommendation_engine
        register_recommendation_engine(&mut defs);

        // Sleep & Recovery — see config::admin::definitions::register_sleep_recovery
        register_sleep_recovery(&mut defs);

        // Training Stress Balance — see config::admin::definitions::register_training_stress_balance
        register_training_stress_balance(&mut defs);

        // Weather Analysis — see config::admin::definitions::register_weather_analysis
        register_weather_analysis(&mut defs);

        // Nutrition — see config::admin::definitions::register_nutrition
        register_nutrition(&mut defs);

        // Tokio Runtime Configuration — see config::admin::definitions::register_tokio_runtime
        register_tokio_runtime(&mut defs);

        // SQLx Connection Pool Configuration — see config::admin::definitions::register_sqlx_pool
        register_sqlx_pool(&mut defs);

        // Cache TTL Configuration — see config::admin::definitions::register_cache_ttl
        register_cache_ttl(&mut defs);

        // Strava Provider Settings — see config::admin::definitions::register_strava_provider
        register_strava_provider(&mut defs);

        // Fitbit Provider Settings — see config::admin::definitions::register_fitbit_provider
        register_fitbit_provider(&mut defs);

        // Garmin Provider Settings — see config::admin::definitions::register_garmin_provider
        register_garmin_provider(&mut defs);

        // MCP Network Settings — see config::admin::definitions::register_mcp_network
        register_mcp_network(&mut defs);

        // Monitoring Thresholds — see config::admin::definitions::register_monitoring
        register_monitoring(&mut defs);

        // Usage Quotas — per-user and per-tenant limits — see config::admin::definitions::register_usage_quotas
        register_usage_quotas(&mut defs);

        // Activity access quotas — separate limits for summary vs detailed mode — see config::admin::definitions::register_activity_access_quotas
        register_activity_access_quotas(&mut defs);

        // LLM Pricing Parameters — see config::admin::definitions::register_llm_pricing
        register_llm_pricing(&mut defs);

        // Group Permissions — see config::admin::definitions::register_group_permissions
        register_group_permissions(&mut defs);

        // Tool Execution — per-turn tool-loop budget — see config::admin::definitions::register_tool_execution
        register_tool_execution(&mut defs);

        defs
    }

    /// Get the full configuration catalog
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the database fails.
    pub async fn get_catalog(
        &self,
        lookup: ConfigLookupScope<'_>,
    ) -> AppResult<ConfigCatalogResponse> {
        // Clone categories and definitions before await to avoid holding locks across await
        let categories = self.categories.read().await.clone();
        let definitions = self.definitions.read().await.clone();
        // Collect every scope the lookup spans, narrowest last is irrelevant —
        // `scope_rank` decides the winner — but all three must be present or a
        // tenant-governed key would be reported as unset for that user.
        let mut overrides = self.manager.get_overrides_at(ConfigScope::Global).await?;
        if let Some(tenant_id) = lookup.tenant_id {
            overrides.extend(
                self.manager
                    .get_overrides_at(ConfigScope::Tenant(tenant_id))
                    .await?,
            );
        }
        if let Some(user_id) = lookup.user_id {
            overrides.extend(
                self.manager
                    .get_overrides_at(ConfigScope::User(user_id))
                    .await?,
            );
        }

        // Build the override lookup. A scoped listing returns the rows at
        // that scope *and* the system-wide rows they layer over, so two rows
        // can share a key — keep the narrower one rather than letting
        // iteration order decide.
        let mut override_map: HashMap<String, &ConfigOverride> = HashMap::new();
        for o in &overrides {
            let full_key = format!("{}.{}", o.category, o.config_key);
            let existing_rank = override_map.get(&full_key).map_or(0, |e| scope_rank(e));
            if scope_rank(o) >= existing_rank {
                override_map.insert(full_key, o);
            }
        }

        let mut result_categories = Vec::new();
        let mut total_params = 0;
        let mut runtime_count = 0;
        let mut static_count = 0;

        for mut category in categories {
            let params: Vec<AdminConfigParameter> = definitions
                .values()
                .filter(|d| d.category == category.name)
                .map(|def| {
                    let full_key = format!("{}.{}", def.category, def.key);
                    let override_val = override_map.get(&full_key);
                    let env_pinned = self.env_pins.get(&def.key);
                    // Same precedence the lookups use: a stored row at this
                    // scope wins, then the environment pin, then the default.
                    let current_value = override_val.map_or_else(
                        || {
                            env_pinned
                                .cloned()
                                .unwrap_or_else(|| def.default_value.clone())
                        },
                        |o| o.config_value.clone(),
                    );
                    let is_modified = override_val.is_some() || env_pinned.is_some();
                    let value_source = override_val.map_or_else(
                        || {
                            if env_pinned.is_some() {
                                "env"
                            } else {
                                "default"
                            }
                        },
                        |o| {
                            if o.user_id.is_some() {
                                "user"
                            } else if o.tenant_id.is_some() {
                                "tenant"
                            } else {
                                "global"
                            }
                        },
                    );

                    total_params += 1;
                    if def.is_runtime_configurable {
                        runtime_count += 1;
                    } else {
                        static_count += 1;
                    }

                    AdminConfigParameter {
                        key: def.key.clone(),
                        display_name: def.display_name.clone(),
                        description: def.description.clone(),
                        category: def.category.clone(),
                        data_type: def.data_type,
                        current_value,
                        default_value: def.default_value.clone(),
                        is_modified,
                        valid_range: def.valid_range.clone(),
                        enum_options: def.enum_options.clone(),
                        units: def.units.clone(),
                        scientific_basis: def.scientific_basis.clone(),
                        env_variable: def.env.as_ref().map(|b| b.name.clone()),
                        env_pinned: env_pinned.is_some(),
                        value_source: value_source.to_owned(),
                        is_runtime_configurable: def.is_runtime_configurable,
                        requires_restart: def.requires_restart,
                    }
                })
                .collect();

            category.parameters = params;
            result_categories.push(category);
        }

        Ok(ConfigCatalogResponse {
            categories: result_categories,
            total_parameters: total_params,
            runtime_configurable_count: runtime_count,
            static_count,
            version: "1.0.0".to_owned(),
        })
    }

    /// Validate configuration values
    pub async fn validate(&self, request: &ValidateConfigRequest) -> ValidateConfigResponse {
        let definitions = self.definitions.read().await;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for (key, value) in &request.parameters {
            if let Some(def) = definitions.get(key) {
                if let Err(error) = Self::validate_value(def, value) {
                    errors.push(*error);
                }

                // Add warnings for non-standard values
                if !def.is_runtime_configurable {
                    warnings.push(format!(
                        "Parameter '{key}' requires server restart to take effect"
                    ));
                }
            } else {
                errors.push(ConfigValidationError {
                    parameter: key.clone(),
                    message: "Unknown configuration parameter".to_owned(),
                    provided_value: value.clone(),
                    valid_range: None,
                });
            }
        }

        ValidateConfigResponse {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    fn validate_value(
        def: &ParameterDefinition,
        value: &serde_json::Value,
    ) -> Result<(), Box<ConfigValidationError>> {
        validate_parameter_value(def, value)
    }
}

/// Audit context threaded through [`AdminConfigService::update_config`].
///
/// Bundles the admin attribution + tenant scope + request metadata so the
/// method doesn't carry a sprawling positional argument list — every caller
/// passes the same combination of identifiers.
pub struct UpdateConfigContext<'a> {
    /// Admin user performing the change (audit attribution).
    pub admin_user_id: &'a str,
    /// Admin email (operator-visible attribution).
    pub admin_email: &'a str,
    /// Which scope the change targets: system-wide, one tenant, or one user.
    pub scope: ConfigScope<'a>,
    /// Client IP captured at request time for forensic tracing.
    pub ip_address: Option<&'a str>,
    /// Client user-agent captured at request time.
    pub user_agent: Option<&'a str>,
}

/// Narrowness rank of a stored row: per-user beats per-tenant beats
/// system-wide. Used to collapse a scoped listing down to one row per key.
const fn scope_rank(o: &ConfigOverride) -> u8 {
    if o.user_id.is_some() {
        2
    } else if o.tenant_id.is_some() {
        1
    } else {
        0
    }
}

impl AdminConfigService {
    /// Update configuration values
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail during update.
    pub async fn update_config(
        &self,
        request: &UpdateConfigRequest,
        ctx: UpdateConfigContext<'_>,
    ) -> AppResult<UpdateConfigResponse> {
        let UpdateConfigContext {
            admin_user_id,
            admin_email,
            scope,
            ip_address,
            user_agent,
        } = ctx;
        // First validate
        let validation = self
            .validate(&ValidateConfigRequest {
                parameters: request.parameters.clone(),
            })
            .await;

        if !validation.is_valid {
            return Ok(UpdateConfigResponse {
                success: false,
                updated_count: 0,
                validation_errors: validation.errors,
                requires_restart: false,
                effective_at: Utc::now(),
                shadowed_by_env: Vec::new(),
            });
        }

        // Clone definitions to avoid holding lock across awaits in the loop
        let definitions = self.definitions.read().await.clone();
        let mut updated_count = 0;
        let mut requires_restart = false;

        for (key, value) in &request.parameters {
            if let Some(def) = definitions.get(key) {
                // Get old value for audit
                let old_override = self.manager.get_override(&def.category, key, scope).await?;
                let old_value = old_override.map(|o| o.config_value);

                // Set the new override
                self.manager
                    .set_override(SetOverrideParams {
                        category: &def.category,
                        key,
                        value,
                        data_type: def.data_type,
                        admin_user_id,
                        scope,
                        reason: request.reason.as_deref(),
                    })
                    .await?;

                // Log the change
                self.manager
                    .log_change(LogChangeParams {
                        admin_user_id,
                        admin_email,
                        category: &def.category,
                        key,
                        old_value: old_value.as_ref(),
                        new_value: value,
                        data_type: def.data_type,
                        reason: request.reason.as_deref(),
                        scope,
                        ip_address,
                        user_agent,
                    })
                    .await?;

                updated_count += 1;
                if def.requires_restart {
                    requires_restart = true;
                }
            }
        }

        let shadowed_by_env = self.shadowed_by_env(&request.parameters, scope);
        for key in &shadowed_by_env {
            // The write succeeded and the row is stored; it simply will not be
            // the value read back while the variable is set. Saying so here is
            // the difference between a no-op and an explained no-op.
            warn!(
                key = %key,
                "Saved a system-wide override that an environment pin outranks; \
                 unset the variable, or scope the override to a tenant or user"
            );
        }

        info!(
            updated_count = updated_count,
            scope = scope.label(),
            shadowed_by_env = shadowed_by_env.len(),
            "Admin updated configuration parameters"
        );

        Ok(UpdateConfigResponse {
            success: true,
            updated_count,
            validation_errors: Vec::new(),
            requires_restart,
            effective_at: Utc::now(),
            shadowed_by_env,
        })
    }

    /// Reset configuration to defaults
    ///
    /// # Errors
    ///
    /// Returns an error if no category is specified or database operations fail.
    pub async fn reset_config(
        &self,
        request: &ResetConfigRequest,
        ctx: UpdateConfigContext<'_>,
    ) -> AppResult<ResetConfigResponse> {
        let Some(category) = request.category.as_deref() else {
            warn!("Reset all configurations requested - this is a destructive operation");
            // Reset all would require iterating all categories
            // For safety, we don't implement "reset everything" without explicit category
            return Err(AppError::invalid_input(
                "Must specify a category to reset. Full reset not supported.",
            ));
        };

        // Clone definitions to avoid holding lock across awaits in the loop
        let definitions = self.definitions.read().await.clone();

        // Validate that the category exists
        let category_exists = definitions.values().any(|def| def.category == category);
        if !category_exists {
            return Err(AppError::not_found(format!(
                "Category '{category}' not found"
            )));
        }

        let (reset_count, reset_keys) = if let Some(keys) = &request.keys {
            self.reset_keys_in_category(
                &definitions,
                category,
                keys,
                request.reason.as_deref(),
                &ctx,
            )
            .await?
        } else {
            self.reset_entire_category(&definitions, category, ctx.scope)
                .await?
        };

        // Refresh cache

        info!(
            reset_count = reset_count,
            "Admin reset configuration parameters"
        );

        Ok(ResetConfigResponse {
            success: true,
            reset_count,
            reset_keys,
        })
    }

    /// Reset a specific list of keys within `category`. Returns the count and
    /// list of keys actually reset (those that had an override).
    async fn reset_keys_in_category(
        &self,
        definitions: &HashMap<String, ParameterDefinition>,
        category: &str,
        keys: &[String],
        reason: Option<&str>,
        ctx: &UpdateConfigContext<'_>,
    ) -> AppResult<(usize, Vec<String>)> {
        let mut reset_count = 0;
        let mut reset_keys = Vec::new();
        for key in keys {
            let Some(def) = definitions.get(key) else {
                continue;
            };
            if def.category != category {
                continue;
            }
            let old_override = self.manager.get_override(category, key, ctx.scope).await?;
            if self
                .manager
                .delete_override(category, key, ctx.scope)
                .await?
            {
                if let Some(old) = old_override {
                    self.manager
                        .log_change(LogChangeParams {
                            admin_user_id: ctx.admin_user_id,
                            admin_email: ctx.admin_email,
                            category,
                            key,
                            old_value: Some(&old.config_value),
                            new_value: &def.default_value,
                            data_type: def.data_type,
                            reason,
                            scope: ctx.scope,
                            ip_address: ctx.ip_address,
                            user_agent: ctx.user_agent,
                        })
                        .await?;
                }
                reset_count += 1;
                reset_keys.push(key.clone());
            }
        }
        Ok((reset_count, reset_keys))
    }

    /// Reset every override under `category`. Returns the count of deleted
    /// rows and the canonical key list for the category.
    async fn reset_entire_category(
        &self,
        definitions: &HashMap<String, ParameterDefinition>,
        category: &str,
        scope: ConfigScope<'_>,
    ) -> AppResult<(usize, Vec<String>)> {
        let reset_count = self
            .manager
            .delete_category_overrides(category, scope)
            .await?;
        let reset_keys = definitions
            .values()
            .filter(|def| def.category == category)
            .map(|def| def.key.clone())
            .collect();
        Ok((reset_count, reset_keys))
    }

    /// Get audit log
    ///
    /// # Errors
    ///
    /// Returns an error if reading the audit log from the database fails.
    pub async fn get_audit_log(
        &self,
        filter: &ConfigAuditFilter,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<ConfigAuditEntry>, usize)> {
        self.manager.get_audit_log(filter, limit, offset).await
    }

    /// Get a specific configuration value
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the database fails.
    pub async fn get_value(
        &self,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>> {
        let definitions = self.definitions.read().await;
        let Some(def) = definitions.get(key) else {
            return Ok(None);
        };
        let category = def.category.clone();
        let default_value = def.default_value.clone();
        drop(definitions);

        if let Some(found) = self.resolve_override(&category, key, scope).await? {
            return Ok(Some(found));
        }
        Ok(Some(default_value))
    }

    /// Read an explicit override for `key` — a stored row or an environment
    /// pin — without falling back to the parameter definition's default.
    ///
    /// Returns `Ok(None)` when nothing overrides the key, so callers can
    /// resolve a domain-specific default (e.g.
    /// [`pierre_core::models::TierQuotaConfig`]) instead of the catalog's
    /// flat default. That flat default would otherwise mask tier-keyed caps:
    /// every tier would read the Starter number.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the database fails.
    pub async fn get_override_value(
        &self,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>> {
        let definitions = self.definitions.read().await;
        let Some(def) = definitions.get(key) else {
            return Ok(None);
        };
        let category = def.category.clone();
        drop(definitions);

        self.resolve_override(&category, key, scope).await
    }

    /// The one place override precedence is defined:
    /// per-user row → per-tenant row → environment pin → system-wide row.
    ///
    /// The environment rung sits above the system-wide row because a pin is a
    /// deploy-time, fleet-wide decision and should beat a runtime admin edit
    /// at the same scope — the same posture `GUARDIAN_*` takes over the
    /// persisted guardian document. A tenant or per-user exemption is
    /// narrower than the fleet, so it still wins over the pin.
    async fn resolve_override(
        &self,
        category: &str,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>> {
        if let Some(user_id) = scope.user_id {
            if let Some(row) = self
                .manager
                .get_override(category, key, ConfigScope::User(user_id))
                .await?
            {
                return Ok(Some(row.config_value));
            }
        }
        if let Some(tenant_id) = scope.tenant_id {
            if let Some(row) = self
                .manager
                .get_override(category, key, ConfigScope::Tenant(tenant_id))
                .await?
            {
                return Ok(Some(row.config_value));
            }
        }
        if let Some(pinned) = self.env_pins.get(key) {
            return Ok(Some(pinned.clone()));
        }
        Ok(self
            .manager
            .get_override(category, key, ConfigScope::Global)
            .await?
            .map(|row| row.config_value))
    }
}
