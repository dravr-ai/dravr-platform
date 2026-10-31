// ABOUTME: Terra provider implementing FitnessProvider trait
// ABOUTME: Serves the activities Terra pushed from the activity cache, bound per connection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Terra `FitnessProvider` implementation
//!
//! This module implements the `FitnessProvider` trait for Terra, enabling
//! Pierre's unified provider interface to work with Terra's webhook-based
//! data delivery model.
//!
//! Terra delivers by pushing: the descriptor declares
//! [`PUSH_ONLY`](crate::spi::ProviderCapabilities::PUSH_ONLY), and every
//! activity read is answered from the rows the activity cache holds under the
//! provider key `terra`, through the stores bound per connection
//! ([`FitnessProvider::bind_owner_stores`]), as for Garmin. A read before
//! the stores are bound is refused, never answered from memory.

use async_trait::async_trait;
use secrecy::ExposeSecret;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

use crate::cached_activity_reads::CachedActivities;
use crate::constants::oauth_providers::TERRA;
use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
    ProviderFactory,
};
use crate::errors::provider::ProviderError;
use crate::errors::{AppError, AppResult};
use crate::models::StatsCoverage;
use crate::models::{Activity, Athlete, Stats};
use crate::pagination::{CursorPage, PaginationParams};
use crate::request_budget;
use crate::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
};

use super::api_client::TerraApiClient;
use super::constants::{
    TERRA_API_BASE_URL, TERRA_DEAUTH_URL, TERRA_TOKEN_URL, TERRA_WIDGET_SESSION_URL,
};

/// What one activity read names when it asks for an activity.
const ACTIVITY: &str = "activity";

/// Terra provider for accessing fitness data from 150+ wearables
///
/// Terra uses a webhook-based model: this provider serves what Terra pushed,
/// from the activity cache rows bound per connection.
#[derive(Debug)]
pub struct TerraProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    stores: OnceLock<OwnerStores>,
    api_client: Option<TerraApiClient>,
    /// Terra user ID for this provider instance
    terra_user_id: RwLock<Option<String>>,
}

impl TerraProvider {
    /// Create a new Terra provider with default configuration
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: ProviderConfig {
                name: "terra".to_owned(),
                auth_url: TERRA_WIDGET_SESSION_URL.to_owned(),
                token_url: TERRA_TOKEN_URL.to_owned(),
                api_base_url: TERRA_API_BASE_URL.to_owned(),
                revoke_url: Some(TERRA_DEAUTH_URL.to_owned()),
                default_scopes: vec![
                    "activity".to_owned(),
                    "sleep".to_owned(),
                    "body".to_owned(),
                    "daily".to_owned(),
                    "nutrition".to_owned(),
                ],
            },
            credentials: RwLock::new(None),
            stores: OnceLock::new(),
            api_client: None,
            terra_user_id: RwLock::new(None),
        }
    }

    /// Create a Terra provider with custom configuration
    #[must_use]
    pub fn with_config(config: ProviderConfig) -> Self {
        Self {
            config,
            credentials: RwLock::new(None),
            stores: OnceLock::new(),
            api_client: None,
            terra_user_id: RwLock::new(None),
        }
    }

    /// The bound stores, or the refusal a read without them gets: Terra
    /// serves no read to fall back on.
    fn stores(&self) -> AppResult<&OwnerStores> {
        self.stores.get().ok_or_else(|| {
            AppError::invalid_input(
                "Terra data is read from what Terra pushed to Dravr, within the tenant \
                 the connection lives in",
            )
        })
    }

    /// The connection's cached Terra activities.
    fn cached(&self) -> AppResult<CachedActivities<'_>> {
        let stores = self.stores()?;
        Ok(CachedActivities {
            cache: stores.activity_cache.as_ref(),
            user_id: stores.user_id,
            tenant_id: &stores.tenant_id,
            provider: TERRA,
        })
    }

    /// Set the Terra user ID for this provider instance
    pub async fn set_terra_user_id(&self, user_id: &str) {
        let mut id = self.terra_user_id.write().await;
        *id = Some(user_id.to_owned());
    }

    /// Get the current Terra user ID
    async fn get_user_id(&self) -> Result<String, ProviderError> {
        let id = self.terra_user_id.read().await;
        id.clone()
            .ok_or_else(|| ProviderError::AuthenticationFailed {
                provider: "terra".to_owned(),
                reason: "Terra user ID not set. Call set_terra_user_id() first.".to_owned(),
            })
    }
}

#[async_trait]
impl FitnessProvider for TerraProvider {
    fn name(&self) -> &'static str {
        "terra"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        // For Terra, credentials contain the API key and dev ID
        // The access_token field stores the Terra user ID
        if let Some(ref token) = credentials.access_token {
            self.set_terra_user_id(token.expose_secret()).await;
        }
        *self.credentials.write().await = Some(credentials);
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        let id = self.terra_user_id.read().await;
        id.is_some()
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // Terra uses API keys, not OAuth tokens that need refreshing
        // The webhook connection stays active as long as the API key is valid
        Ok(())
    }

    fn bind_owner_stores(&self, stores: OwnerStores) {
        if self.stores.set(stores).is_err() {
            debug!("Terra owner stores already bound; keeping the first");
        }
    }

    #[instrument(skip(self), fields(provider = "terra", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let user_id = self.get_user_id().await?;

        // If we have an API client, fetch user info
        if let Some(ref client) = self.api_client {
            // The signing app's budget admits the request before it is sent.
            let budget = request_budget::carried_by(&self.credentials).await;
            request_budget::admit(budget.as_ref(), "terra").await?;
            let user_info = client.get_user_info(&user_id).await?;
            if let Some(user) = user_info.user {
                return Ok(Athlete {
                    id: user.user_id,
                    username: user.reference_id.unwrap_or_default(),
                    firstname: None,
                    lastname: None,
                    profile_picture: None,
                    provider: format!("terra:{}", user.provider.to_lowercase()),
                    sex: None,
                });
            }
        }

        // Return basic athlete info from user ID
        Ok(Athlete {
            id: user_id.clone(),
            username: user_id,
            firstname: None,
            lastname: None,
            profile_picture: None,
            provider: "terra".to_owned(),
            sex: None,
        })
    }

    #[instrument(
        skip(self, params),
        fields(
            provider = "terra",
            api_call = "get_activities",
            limit = ?params.limit,
            offset = ?params.offset,
        )
    )]
    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        self.cached()?.window(params).await
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        self.cached()?.cursor_page(params).await
    }

    #[instrument(
        skip(self),
        fields(provider = "terra", api_call = "get_activity", activity_id = %id)
    )]
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        self.cached()?.one(id, ACTIVITY).await
    }

    /// Totals over the pushed activities Dravr holds. Terra pushes from the
    /// connection onward, so these cover what was pushed, never a lifetime.
    #[instrument(skip(self), fields(provider = "terra", api_call = "get_stats"))]
    async fn get_stats(&self) -> AppResult<Stats> {
        let activities = self.cached()?.all().await?;
        Ok(Stats {
            total_activities: u64::try_from(activities.len()).unwrap_or(u64::MAX),
            total_distance: activities
                .iter()
                .filter_map(Activity::distance_meters)
                .sum(),
            total_duration: activities.iter().map(Activity::duration_seconds).sum(),
            total_elevation_gain: activities.iter().filter_map(Activity::elevation_gain).sum(),
            coverage: StatsCoverage::recent(&activities),
            year_to_date: None,
        })
    }
}

impl Default for TerraProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Terra provider descriptor for SPI
#[derive(Debug)]
pub struct TerraDescriptor;

impl ProviderDescriptor for TerraDescriptor {
    fn name(&self) -> &'static str {
        "terra"
    }

    fn display_name(&self) -> &'static str {
        "Terra"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // LIMITATION(registre#513): `TerraDescriptor::capabilities` advertises activities
        // only. The webhook files Terra's nights, recovery and body days into the wellness
        // store (`services::terra_ingest`), where the readers find them like any provider's
        // rows, but the webhook route is not mounted (registre#886): no Terra wellness row
        // reaches a deployment, and advertising sleep or recovery would promise data that
        // never arrives.
        //
        // No `RECORDED_ACTIVITIES`: Terra relays whichever wearable the athlete
        // linked through it, heart-rate-detected workouts included, so it cannot
        // vouch that an activity is a recording.
        //
        // `PUSH_ONLY`: Terra pushes every activity to Dravr and serves no list
        // read, so the reads answer from the rows the push stored, and no
        // write-through restamps them as fresh.
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::PUSH_ONLY)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        // Terra uses API key auth + widget sessions, not traditional OAuth
        // Returning endpoints for widget session generation
        Some(OAuthEndpoints {
            auth_url: TERRA_WIDGET_SESSION_URL,
            token_url: TERRA_TOKEN_URL,
            revoke_url: Some(TERRA_DEAUTH_URL),
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            scope_separator: ",",
            use_pkce: false, // Terra uses API keys
            additional_auth_params: &[],
        })
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // Terra authenticates with an API key; there is no refresh grant
    }

    fn api_base_url(&self) -> &'static str {
        TERRA_API_BASE_URL
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &["activity", "sleep", "body", "daily", "nutrition"]
    }
}

/// Factory for creating Terra provider instances
#[derive(Debug, Default)]
pub struct TerraProviderFactory;

impl ProviderFactory for TerraProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(TerraProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["terra"]
    }
}
