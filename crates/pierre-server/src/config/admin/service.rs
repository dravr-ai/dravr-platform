// ABOUTME: Configuration service with caching and hot reload support
// ABOUTME: Provides runtime configuration access with database override resolution
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::definitions::{
    register_activity_access_quotas, register_algorithm_selection, register_cache_ttl,
    register_feature_flags, register_fitbit_provider, register_garmin_provider,
    register_heart_rate_zones, register_llm_pricing, register_llm_provider_config,
    register_mcp_network, register_monitoring, register_nutrition, register_rate_limiting,
    register_recommendation_engine, register_sleep_recovery, register_sqlx_pool,
    register_strava_provider, register_tokio_runtime, register_training_stress_balance,
    register_usage_quotas, register_weather_analysis,
};
use super::manager::AdminConfigManager;
use super::repository::{AdminConfigRepository, LogChangeParams, SetOverrideParams};
use super::types::{
    AdminConfigCategory, AdminConfigParameter, ConfigAuditFilter, ConfigCatalogResponse,
    ConfigDataType, ConfigOverride, ConfigValidationError, ParameterRange, ResetConfigRequest,
    ResetConfigResponse, UpdateConfigRequest, UpdateConfigResponse, ValidateConfigRequest,
    ValidateConfigResponse,
};
use crate::errors::{AppError, AppResult};
use chrono::Utc;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Default configuration definitions with metadata
/// This struct holds the canonical parameter definitions loaded at startup
#[derive(Debug, Clone)]
pub struct ParameterDefinition {
    /// Unique identifier for the parameter (e.g., `rate_limit.free_tier_burst`)
    pub key: String,
    /// Human-readable name for display in UI
    pub display_name: String,
    /// Detailed description of what this parameter controls
    pub description: String,
    /// Category grouping for organization (e.g., `rate_limiting`, `algorithms`)
    pub category: String,
    /// Data type for validation and UI rendering
    pub data_type: ConfigDataType,
    /// Default value when no override is set
    pub default_value: serde_json::Value,
    /// Optional numeric range constraints for validation
    pub valid_range: Option<ParameterRange>,
    /// Optional list of valid enum values
    pub enum_options: Option<Vec<String>>,
    /// Unit of measurement for display (e.g., "requests", "km", "% max HR")
    pub units: Option<String>,
    /// Scientific or research basis for the default value
    pub scientific_basis: Option<String>,
    /// Environment variable name if this can be set via env
    pub env_variable: Option<String>,
    /// Whether this can be changed at runtime without restart
    pub is_runtime_configurable: bool,
    /// Whether changing this parameter requires a server restart
    pub requires_restart: bool,
}

/// Admin configuration service for managing runtime configuration
pub struct AdminConfigService {
    manager: Box<dyn AdminConfigRepository>,
    /// Cached parameter definitions (loaded at startup)
    definitions: Arc<RwLock<HashMap<String, ParameterDefinition>>>,
    /// Cached effective values (refreshed on changes)
    cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
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

        let service = Self {
            manager,
            definitions: Arc::new(RwLock::new(HashMap::new())),
            cache: Arc::new(RwLock::new(HashMap::new())),
            categories: Arc::new(RwLock::new(categories)),
        };

        // Initialize parameter definitions
        service.initialize_definitions().await;

        // Load overrides into cache
        service.refresh_cache(None).await?;

        Ok(service)
    }

    /// Initialize parameter definitions with all configurable parameters
    async fn initialize_definitions(&self) {
        // Build definitions locally first, then acquire lock briefly at the end
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

        // Acquire lock briefly and insert all definitions at once
        let def_count = defs.len();
        self.definitions.write().await.extend(defs);

        info!("Initialized {def_count} admin configuration parameter definitions");
    }

    /// Refresh the cache from database overrides
    ///
    /// # Errors
    ///
    /// Returns an error if reading overrides from the database fails.
    pub async fn refresh_cache(&self, tenant_id: Option<&str>) -> AppResult<()> {
        let overrides = self.manager.get_overrides(tenant_id).await?;

        // Build the new cache entries
        let new_entries: HashMap<String, serde_json::Value> = overrides
            .into_iter()
            .map(|o| {
                let key = format!("{}.{}", o.category, o.config_key);
                (key, o.config_value)
            })
            .collect();

        let entry_count = new_entries.len();

        // Update the cache with new entries
        {
            let mut cache = self.cache.write().await;
            cache.clear();
            cache.extend(new_entries);
        }

        debug!("Refreshed config cache with {entry_count} overrides");
        Ok(())
    }

    /// Get the full configuration catalog
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the database fails.
    pub async fn get_catalog(&self, tenant_id: Option<&str>) -> AppResult<ConfigCatalogResponse> {
        // Clone categories and definitions before await to avoid holding locks across await
        let categories = self.categories.read().await.clone();
        let definitions = self.definitions.read().await.clone();
        let overrides = self.manager.get_overrides(tenant_id).await?;

        // Build override lookup
        let override_map: HashMap<String, &ConfigOverride> = overrides
            .iter()
            .map(|o| (format!("{}.{}", o.category, o.config_key), o))
            .collect();

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
                    let current_value = override_val
                        .map_or_else(|| def.default_value.clone(), |o| o.config_value.clone());
                    let is_modified = override_val.is_some();

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
                        env_variable: def.env_variable.clone(),
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
        match def.data_type {
            ConfigDataType::Float => {
                let num = value.as_f64().ok_or_else(|| {
                    Box::new(ConfigValidationError {
                        parameter: def.key.clone(),
                        message: "Expected a floating point number".to_owned(),
                        provided_value: value.clone(),
                        valid_range: def.valid_range.clone(),
                    })
                })?;

                if let Some(range) = &def.valid_range {
                    let min = range.min.as_f64().unwrap_or(f64::MIN);
                    let max = range.max.as_f64().unwrap_or(f64::MAX);
                    if num < min || num > max {
                        return Err(Box::new(ConfigValidationError {
                            parameter: def.key.clone(),
                            message: format!("Value must be between {min} and {max}"),
                            provided_value: value.clone(),
                            valid_range: Some(range.clone()),
                        }));
                    }
                }
            }
            ConfigDataType::Integer => {
                let num = value.as_i64().ok_or_else(|| {
                    Box::new(ConfigValidationError {
                        parameter: def.key.clone(),
                        message: "Expected an integer".to_owned(),
                        provided_value: value.clone(),
                        valid_range: def.valid_range.clone(),
                    })
                })?;

                if let Some(range) = &def.valid_range {
                    let min = range.min.as_i64().unwrap_or(i64::MIN);
                    let max = range.max.as_i64().unwrap_or(i64::MAX);
                    if num < min || num > max {
                        return Err(Box::new(ConfigValidationError {
                            parameter: def.key.clone(),
                            message: format!("Value must be between {min} and {max}"),
                            provided_value: value.clone(),
                            valid_range: Some(range.clone()),
                        }));
                    }
                }
            }
            ConfigDataType::Boolean => {
                if !value.is_boolean() {
                    return Err(Box::new(ConfigValidationError {
                        parameter: def.key.clone(),
                        message: "Expected a boolean (true/false)".to_owned(),
                        provided_value: value.clone(),
                        valid_range: None,
                    }));
                }
            }
            ConfigDataType::String => {
                if !value.is_string() {
                    return Err(Box::new(ConfigValidationError {
                        parameter: def.key.clone(),
                        message: "Expected a string".to_owned(),
                        provided_value: value.clone(),
                        valid_range: None,
                    }));
                }
            }
            ConfigDataType::Enum => {
                let str_val = value.as_str().ok_or_else(|| {
                    Box::new(ConfigValidationError {
                        parameter: def.key.clone(),
                        message: "Expected a string value for enum".to_owned(),
                        provided_value: value.clone(),
                        valid_range: None,
                    })
                })?;

                if let Some(options) = &def.enum_options {
                    if !options.contains(&str_val.to_owned()) {
                        return Err(Box::new(ConfigValidationError {
                            parameter: def.key.clone(),
                            message: format!("Value must be one of: {}", options.join(", ")),
                            provided_value: value.clone(),
                            valid_range: None,
                        }));
                    }
                }
            }
        }

        Ok(())
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
    /// Tenant scope; `None` records a system-wide change.
    pub tenant_id: Option<&'a str>,
    /// Client IP captured at request time for forensic tracing.
    pub ip_address: Option<&'a str>,
    /// Client user-agent captured at request time.
    pub user_agent: Option<&'a str>,
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
            tenant_id,
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
            });
        }

        // Clone definitions to avoid holding lock across awaits in the loop
        let definitions = self.definitions.read().await.clone();
        let mut updated_count = 0;
        let mut requires_restart = false;

        for (key, value) in &request.parameters {
            if let Some(def) = definitions.get(key) {
                // Get old value for audit
                let old_override = self
                    .manager
                    .get_override(&def.category, key, tenant_id)
                    .await?;
                let old_value = old_override.map(|o| o.config_value);

                // Set the new override
                self.manager
                    .set_override(SetOverrideParams {
                        category: &def.category,
                        key,
                        value,
                        data_type: def.data_type,
                        admin_user_id,
                        tenant_id,
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
                        tenant_id,
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

        // Refresh cache
        self.refresh_cache(tenant_id).await?;

        info!(
            updated_count = updated_count,
            "Admin updated configuration parameters"
        );

        Ok(UpdateConfigResponse {
            success: true,
            updated_count,
            validation_errors: Vec::new(),
            requires_restart,
            effective_at: Utc::now(),
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
            self.reset_entire_category(&definitions, category, ctx.tenant_id)
                .await?
        };

        // Refresh cache
        self.refresh_cache(ctx.tenant_id).await?;

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
            let old_override = self
                .manager
                .get_override(category, key, ctx.tenant_id)
                .await?;
            if self
                .manager
                .delete_override(category, key, ctx.tenant_id)
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
                            tenant_id: ctx.tenant_id,
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
        tenant_id: Option<&str>,
    ) -> AppResult<(usize, Vec<String>)> {
        let reset_count = self
            .manager
            .delete_category_overrides(category, tenant_id)
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
    ) -> AppResult<(Vec<super::types::ConfigAuditEntry>, usize)> {
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
        tenant_id: Option<&str>,
    ) -> AppResult<Option<serde_json::Value>> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(val) = cache.get(key) {
                return Ok(Some(val.clone()));
            }
        }

        // Get definition to find category and default
        let definitions = self.definitions.read().await;
        if let Some(def) = definitions.get(key) {
            let category = def.category.clone();
            let default_value = def.default_value.clone();
            drop(definitions); // Release lock before await

            // Check database
            if let Some(override_val) = self
                .manager
                .get_effective_override(&category, key, tenant_id)
                .await?
            {
                return Ok(Some(override_val.config_value));
            }

            // Return default
            return Ok(Some(default_value));
        }

        Ok(None)
    }

    /// Read an explicit override (cache + `admin_config_overrides` row)
    /// without falling back to the parameter definition's default.
    ///
    /// Returns `Ok(None)` when no override exists, so the caller can
    /// resolve a domain-specific default (e.g.
    /// [`pierre_core::models::TierQuotaConfig`]) instead of the
    /// catalog's flat default. This is the canonical entry point for
    /// quota lookups — the [`Self::get_value`] flat-default behaviour
    /// would otherwise mask tier-keyed caps.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the database fails.
    pub async fn get_override_value(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<serde_json::Value>> {
        {
            let cache = self.cache.read().await;
            if let Some(val) = cache.get(key) {
                return Ok(Some(val.clone()));
            }
        }
        let definitions = self.definitions.read().await;
        let category = match definitions.get(key) {
            Some(def) => def.category.clone(),
            None => return Ok(None),
        };
        drop(definitions);
        let override_row = self
            .manager
            .get_effective_override(&category, key, tenant_id)
            .await?;
        Ok(override_row.map(|o| o.config_value))
    }
}
