// ABOUTME: Garmin Connect Developer Program provider: OAuth2 PKCE identity and refresh, data read from what Garmin pushed
// ABOUTME: Activities, laps and samples come from Dravr's own stores; only `user/id` and the token endpoint are called
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Garmin, read from what Garmin pushed
//!
//! The Garmin Health and Activity APIs at the Starter tier deliver data only
//! by pushing it: every summary is POSTed to Dravr as it is recorded, and no
//! list, detail or backfill read exists (carnet#1155). The descriptor declares
//! [`PUSH_ONLY`](crate::spi::ProviderCapabilities::PUSH_ONLY), and this
//! provider serves every data read from the stores the push ingest writes,
//! bound per connection through [`FitnessProvider::bind_owner_stores`]. The
//! only calls it makes to Garmin are `GET user/id` ([`FitnessProvider::get_athlete`])
//! and the token refresh; deregistration is the descriptor's revoke URL.
//!
//! ## Where the push ingest writes, and so where this reads
//!
//! - **An activity summary** (`activities`) is one row of `cached_activities`
//!   under the provider key `garmin`, its id the decimal Garmin `activityId`,
//!   its `source` `garmin` (the attribution keys on it, carnet#1154), written
//!   with `synced_at` = the push's receipt time. A re-sent summary upserts the
//!   same row. Freshness is the last push: `latest_activity_sync` reads the
//!   later of the rows' `synced_at` and the fetch mark, so a push that carried
//!   no new activity records `record_activity_fetch` at its receipt time.
//! - **Laps** from an activity's details (`activityDetails`) go beside that row
//!   through `store_activity_detail`, as the laps of a detail read do for
//!   every provider; every read of the row fills them in. The row must exist
//!   first, so the details' own summary is upserted before them when the
//!   activity summary has not arrived yet.
//! - **Samples** from the same details go to `activity_streams`
//!   (`upsert_activity_streams`, origin `push`), as the [`TimeSeriesData`]
//!   the activity model carries, keyed like the cached row, and are read back
//!   through the one store-backed sample read on [`OwnerStores`] every
//!   provider read from Dravr's stores shares. Not inside the row's `data_json`:
//!   every list read deserializes that column, and a later summary push
//!   rewrites it whole. An activity longer than 24 h, which the details leave
//!   out, has its samples written there from its FIT file.
//!
//! A read before the stores are bound — no tenant to read in — is refused,
//! never answered from Garmin.

use super::circuit_breaker::CircuitBreaker;
use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, OwnerStores, ProviderConfig,
    TokenRefreshCallback,
};
use super::errors::provider::ProviderError;
use super::utils::{self, RetryConfig};
use crate::cached_activity_reads::CachedActivities;
use crate::constants::oauth::{
    GARMIN_API_BASE_URL, GARMIN_AUTH_URL, GARMIN_DEREGISTRATION_URL, GARMIN_TOKEN_URL,
};
use crate::constants::{api_provider_limits, oauth_providers};
use crate::errors::{AppError, AppResult};
use crate::http_client::{shared_client, SharedHttpClient};
use crate::models::refresh_due;
use crate::models::{Activity, Athlete, Stats, StatsCoverage, TimeSeriesData};
use crate::pagination::{CursorPage, PaginationParams};
use crate::request_budget;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::StatusCode;
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::fmt;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};

mod wire;

use wire::GarminUserIdResponse;

/// What an activity is called in a read's errors.
const ACTIVITY: &str = "Garmin activity";

/// Garmin Connect Developer Program provider implementation
pub struct GarminProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    client: SharedHttpClient,
    circuit_breaker: CircuitBreaker,
    token_refresh_callback: OnceLock<TokenRefreshCallback>,
    stores: OnceLock<OwnerStores>,
}

impl fmt::Debug for GarminProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GarminProvider")
            .field("config", &self.config)
            .field("credentials", &self.credentials)
            .field("client", &self.client)
            .field("circuit_breaker", &self.circuit_breaker)
            .field("stores", &self.stores.get())
            .finish_non_exhaustive()
    }
}

impl GarminProvider {
    /// Create a new Garmin provider with default configuration
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(ProviderConfig {
            name: oauth_providers::GARMIN.to_owned(),
            auth_url: GARMIN_AUTH_URL.to_owned(),
            token_url: GARMIN_TOKEN_URL.to_owned(),
            api_base_url: GARMIN_API_BASE_URL.to_owned(),
            revoke_url: Some(GARMIN_DEREGISTRATION_URL.to_owned()),
            // Garmin's scope is fixed server-side; its authorization takes none.
            default_scopes: Vec::new(),
        })
    }

    /// Create provider with custom configuration
    #[must_use]
    pub fn with_config(config: ProviderConfig) -> Self {
        let provider_name = config.name.clone();
        Self {
            circuit_breaker: CircuitBreaker::new(&provider_name),
            config,
            credentials: RwLock::new(None),
            // Clone Arc<Client> from shared singleton - cheap reference counting operation
            client: shared_client().clone(),
            token_refresh_callback: OnceLock::new(),
            stores: OnceLock::new(),
        }
    }

    /// The bound stores, or the refusal a read without them gets: Garmin
    /// serves no read to fall back on.
    fn stores(&self) -> AppResult<&OwnerStores> {
        self.stores.get().ok_or_else(|| {
            AppError::invalid_input(
                "Garmin data is read from what Garmin pushed to Dravr, within the tenant \
                 the connection lives in",
            )
        })
    }

    /// The connection's cached Garmin activities.
    fn cached(&self) -> AppResult<CachedActivities<'_>> {
        let stores = self.stores()?;
        Ok(CachedActivities {
            cache: stores.activity_cache.as_ref(),
            user_id: stores.user_id,
            tenant_id: &stores.tenant_id,
            provider: oauth_providers::GARMIN,
        })
    }

    /// Make authenticated API request with rate limit handling and circuit breaker protection
    /// Uses shared retry logic with exponential backoff for 429 errors
    async fn api_request<T>(&self, endpoint: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        // Check circuit breaker before making request
        if !self.circuit_breaker.is_allowed() {
            let err = ProviderError::CircuitBreakerOpen {
                provider: oauth_providers::GARMIN.to_owned(),
                retry_after_secs: 30,
            };
            return Err(AppError::external_service("Garmin", err.to_string()));
        }

        self.refresh_token_if_needed().await?;

        // Clone access token to avoid holding lock across await
        let access_token = {
            let guard = self.credentials.read().await;
            let credentials = guard.as_ref().ok_or_else(|| {
                AppError::internal("No credentials available for Garmin API request")
            })?;

            let token = credentials
                .access_token
                .clone() // Safe: String ownership needed for async request
                .ok_or_else(|| AppError::internal("No access token available"))?;
            drop(guard); // Release lock immediately after cloning
            token
        };

        let url = format!(
            "{}/{}",
            self.config.api_base_url,
            endpoint.trim_start_matches('/')
        );

        let retry_config = RetryConfig {
            max_retries: 3,
            initial_backoff_ms: 1000,
            retryable_status_codes: vec![StatusCode::TOO_MANY_REQUESTS],
            estimated_block_duration_secs:
                api_provider_limits::garmin::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS,
        };

        // The signing app's budget, which admits each request before it is sent.
        let budget = request_budget::carried_by(&self.credentials).await;
        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            access_token.expose_secret(),
            oauth_providers::GARMIN,
            &retry_config,
            budget.as_ref(),
            utils::no_vendor_error,
        )
        .await;

        // Record success/failure for circuit breaker
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(e) => self.circuit_breaker.record_error(e),
        }

        result
    }
}

impl Default for GarminProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FitnessProvider for GarminProvider {
    fn name(&self) -> &'static str {
        oauth_providers::GARMIN
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        info!("Setting Garmin credentials");
        *self.credentials.write().await = Some(credentials);
        Ok(())
    }

    fn set_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        if self.token_refresh_callback.set(callback).is_err() {
            debug!("Garmin token refresh callback already set; keeping the first");
        }
    }

    fn bind_owner_stores(&self, stores: OwnerStores) {
        if self.stores.set(stores).is_err() {
            debug!("Garmin owner stores already bound; keeping the first");
        }
    }

    async fn is_authenticated(&self) -> bool {
        if let Some(creds) = self.credentials.read().await.as_ref() {
            if creds.access_token.is_some() {
                // Check if token is expired
                if let Some(expires_at) = creds.expires_at {
                    return Utc::now() < expires_at;
                }
                return true;
            }
        }
        false
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // Check if refresh is needed and extract credentials
        let (needs_refresh, credentials) = {
            let guard = self.credentials.read().await;
            let needs_refresh = if let Some(creds) = guard.as_ref() {
                creds.expires_at.is_some_and(refresh_due)
            } else {
                return Err(AppError::internal("No credentials available"));
            };

            let credentials = guard
                .as_ref()
                .ok_or_else(|| AppError::internal("No credentials available for refresh"))?
                .clone(); // Safe: OAuth2Credentials ownership for refresh operation
            drop(guard); // Release lock early to avoid contention

            (needs_refresh, credentials)
        };

        if !needs_refresh {
            return Ok(());
        }

        let refresh_token = credentials
            .refresh_token
            .ok_or_else(|| AppError::internal("No refresh token available"))?;

        // Garmin's refresh grant, as its descriptor declares it: the client
        // credentials in the form body and no other field. Garmin rotates the
        // refresh token on every refresh, so the new one is what the
        // write-back callback stores.
        let mut new_credentials = utils::refresh_oauth_token(
            &self.client,
            &utils::RefreshRequest::form_fields(
                oauth_providers::GARMIN,
                &self.config.token_url,
                &credentials.client_id,
                credentials.client_secret.expose_secret(),
                refresh_token.expose_secret(),
            ),
        )
        .await?;

        // Preserve original scopes
        new_credentials.scopes = credentials.scopes;
        // A refresh keeps the app that signs the token, so its budget carries over.
        new_credentials.request_budget = credentials.request_budget;

        *self.credentials.write().await = Some(new_credentials.clone());

        // Persist refreshed token to database via callback
        if let Some(callback) = self.token_refresh_callback.get() {
            callback(new_credentials).await;
        }

        Ok(())
    }

    /// The Garmin user behind the access token.
    ///
    /// The Health API serves the user's id at `GET user/id` and nothing else
    /// about them (no name, no picture), so the athlete is the id alone. It is
    /// what Garmin's push notifications name the user by, and what the OAuth
    /// callback stores as the connection's owner id.
    #[instrument(skip(self), fields(provider = "garmin", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let user: GarminUserIdResponse = self.api_request("user/id").await?;

        Ok(Athlete {
            id: user.user_id,
            username: String::new(),
            firstname: None,
            lastname: None,
            profile_picture: None,
            provider: oauth_providers::GARMIN.to_owned(),
            sex: None,
        })
    }

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

    /// The pushed summary, with the laps its details carried filled in by
    /// the cache read; there is no richer read to make, so this also answers
    /// [`FitnessProvider::get_activity_detailed`].
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        self.cached()?.one(id, ACTIVITY).await
    }

    fn serves_activity_streams(&self) -> bool {
        true
    }

    /// The pushed summary with the samples its details carried. An activity
    /// whose details have not arrived is served without samples, which says
    /// nothing about whether it recorded any.
    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        self.stores()?
            .stored_activity_with_streams(self.name(), id, ACTIVITY)
            .await
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        self.stores()?
            .stored_activity_streams(self.name(), id)
            .await
    }

    /// Totals over the pushed activities Dravr holds. Garmin pushes from the
    /// connection onward and serves no aggregate, so these cover what was
    /// pushed, never a lifetime.
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

// ============================================================================
// Provider Factory
// ============================================================================

use super::core::ProviderFactory;

/// Factory for creating Garmin provider instances
#[derive(Debug)]
pub struct GarminProviderFactory;

impl ProviderFactory for GarminProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(GarminProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[oauth_providers::GARMIN]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActivityBuilder, SportType, TenantId};
    use crate::spi::{GarminDescriptor, ProviderCapabilities, ProviderDescriptor};
    use chrono::{DateTime, Duration};
    use pierre_database::repositories::{ActivityStreamsWrite, StreamOrigin};
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;
    use std::sync::Arc;
    use uuid::Uuid;

    /// One athlete's stores, with a Garmin ride and run pushed, a Strava ride
    /// of their own, and another athlete's Garmin ride beside them.
    struct Seeded {
        stores: OwnerStores,
        ride_start: DateTime<Utc>,
    }

    fn activity(id: &str, sport: SportType, start: DateTime<Utc>, provider: &str) -> Activity {
        ActivityBuilder::new(id, "Pushed", sport, start, 3_600, provider)
            .source(provider)
            .distance_meters(30_000.0)
            .elevation_gain(250.0)
            .build()
    }

    async fn seeded() -> Seeded {
        let db = create_test_db().await.expect("test db");
        let repos = db.repositories();
        let tenant = TenantId::from_uuid(Uuid::new_v4());
        let athlete = create_test_user(&format!("{}@garmin.test", Uuid::new_v4()), None);
        let stranger = create_test_user(&format!("{}@garmin.test", Uuid::new_v4()), None);
        repos.users.create(&athlete).await.expect("athlete");
        repos.users.create(&stranger).await.expect("stranger");

        let ride_start = Utc::now() - Duration::days(2);
        let run_start = Utc::now() - Duration::days(1);
        repos
            .activity_cache
            .upsert_activities(
                athlete.id,
                &tenant,
                oauth_providers::GARMIN,
                &[
                    activity("1001", SportType::Ride, ride_start, oauth_providers::GARMIN),
                    activity("1002", SportType::Run, run_start, oauth_providers::GARMIN),
                ],
            )
            .await
            .expect("seed garmin");
        repos
            .activity_cache
            .upsert_activities(
                athlete.id,
                &tenant,
                oauth_providers::STRAVA,
                &[activity(
                    "9",
                    SportType::Ride,
                    run_start,
                    oauth_providers::STRAVA,
                )],
            )
            .await
            .expect("seed strava");
        repos
            .activity_cache
            .upsert_activities(
                stranger.id,
                &tenant,
                oauth_providers::GARMIN,
                &[activity(
                    "2001",
                    SportType::Ride,
                    ride_start,
                    oauth_providers::GARMIN,
                )],
            )
            .await
            .expect("seed stranger");

        Seeded {
            stores: OwnerStores {
                activity_cache: Arc::clone(&repos.activity_cache),
                activity_streams: Arc::clone(&repos.activity_streams),
                user_id: athlete.id,
                tenant_id: tenant,
            },
            ride_start,
        }
    }

    fn bound(stores: OwnerStores) -> GarminProvider {
        let provider = GarminProvider::new();
        provider.bind_owner_stores(stores);
        provider
    }

    #[test]
    fn the_garmin_descriptor_is_push_only() {
        assert!(
            GarminDescriptor.capabilities().is_push_only(),
            "bug: the Garmin descriptor must declare PUSH_ONLY"
        );
        assert!(
            !ProviderCapabilities::full_health().is_push_only(),
            "bug: full_health must not imply PUSH_ONLY"
        );
        assert!(
            !ProviderCapabilities::activity_only().is_push_only(),
            "bug: activity_only must not imply PUSH_ONLY"
        );
    }

    #[tokio::test]
    async fn an_unbound_provider_refuses_every_data_read() {
        let provider = GarminProvider::new();
        for result in [
            provider.get_activities(Some(10), None).await.map(|_| ()),
            provider.get_activity("1001").await.map(|_| ()),
            provider.get_stats().await.map(|_| ()),
            provider.get_activity_streams("1001").await.map(|_| ()),
        ] {
            let err = result.expect_err("no stores, no read");
            assert!(err.to_string().contains("pushed"), "{err}");
        }
    }

    #[tokio::test]
    async fn the_list_is_the_athletes_own_pushed_garmin_activities_newest_first() {
        let seeded = seeded().await;
        let provider = bound(seeded.stores);

        let listed = provider.get_activities(None, None).await.expect("list");
        let ids: Vec<&str> = listed.iter().map(Activity::id).collect();
        assert_eq!(ids, ["1002", "1001"], "own Garmin rows only, newest first");
        assert!(
            listed
                .iter()
                .all(|a| a.source() == Some(oauth_providers::GARMIN)),
            "bug: every listed row must carry the Garmin source"
        );

        let older = provider
            .get_activities_with_params(&ActivityQueryParams::with_time_range(
                Some((seeded.ride_start + Duration::hours(1)).timestamp()),
                None,
            ))
            .await
            .expect("bounded list");
        assert_eq!(
            older.iter().map(Activity::id).collect::<Vec<_>>(),
            ["1001"],
            "bug: the time window must bound the list"
        );

        let page = provider
            .get_activities_cursor(&PaginationParams::forward(None, 1))
            .await
            .expect("cursor page");
        assert_eq!(page.items.len(), 1, "bug: the page must honour its size");
        assert!(page.has_more, "bug: a second row must be announced");
    }

    #[tokio::test]
    async fn one_activity_is_read_from_the_cache_and_an_unknown_id_is_not_found() {
        let provider = bound(seeded().await.stores);

        let ride = provider.get_activity("1001").await.expect("cached ride");
        assert_eq!(*ride.sport_type(), SportType::Ride, "bug: the cached sport");
        let detailed = provider
            .get_activity_detailed("1001")
            .await
            .expect("detail");
        assert_eq!(
            detailed.id(),
            "1001",
            "bug: the detail read is the cached row"
        );

        let err = provider
            .get_activity("2001")
            .await
            .expect_err("a stranger's");
        assert!(err.to_string().contains("2001"), "{err}");
    }

    #[tokio::test]
    async fn stored_samples_are_served_with_the_activity_and_absent_ones_are_none() {
        let seeded = seeded().await;
        let stores = seeded.stores.clone();
        let samples = TimeSeriesData {
            timestamps: vec![0, 1, 2],
            heart_rate: Some(vec![Some(120), None, Some(124)]),
            power: Some(vec![Some(210), Some(215), Some(220)]),
            cadence: None,
            speed: None,
            altitude: None,
            temperature: None,
            gps_coordinates: Some(vec![(45.5, -73.6), (45.5001, -73.6), (45.5002, -73.6)]),
            distance: None,
        };
        stores
            .activity_streams
            .upsert_activity_streams(&ActivityStreamsWrite {
                tenant_id: &stores.tenant_id,
                user_id: stores.user_id,
                provider: oauth_providers::GARMIN,
                activity_id: "1001",
                origin: StreamOrigin::Push,
                source_ref: None,
                streams: &samples,
                synced_at: Utc::now(),
            })
            .await
            .expect("store samples");
        let provider = bound(seeded.stores);

        assert!(
            provider.serves_activity_streams(),
            "bug: pushed samples are a sample source"
        );
        let ride = provider
            .get_activity_with_streams("1001")
            .await
            .expect("ride with samples");
        assert_eq!(
            ride.time_series_data(),
            Some(&samples),
            "bug: the stored samples ride with the activity"
        );
        assert_eq!(ride.distance_meters(), Some(30_000.0), "the summary stands");
        assert_eq!(
            provider
                .get_activity_streams("1001")
                .await
                .expect("samples"),
            Some(samples),
            "bug: the stored samples are read back"
        );

        let run = provider
            .get_activity_with_streams("1002")
            .await
            .expect("run without samples");
        assert!(
            run.time_series_data().is_none(),
            "bug: no stored samples, none served"
        );
        assert_eq!(
            provider.get_activity_streams("1002").await.expect("read"),
            None,
            "bug: no stored samples, none served"
        );
    }

    /// The store-backed read is keyed by the provider slug the activity is
    /// read under: samples another provider key holds for the same id are
    /// never served as Garmin's.
    #[tokio::test]
    async fn samples_stored_under_another_provider_key_are_not_garmins() {
        let seeded = seeded().await;
        let stores = seeded.stores.clone();
        let samples = TimeSeriesData {
            timestamps: vec![0, 1],
            heart_rate: Some(vec![Some(120), Some(121)]),
            power: None,
            cadence: None,
            speed: None,
            altitude: None,
            temperature: None,
            gps_coordinates: None,
            distance: None,
        };
        let written = stores
            .activity_streams
            .upsert_activity_streams(&ActivityStreamsWrite {
                tenant_id: &stores.tenant_id,
                user_id: stores.user_id,
                provider: oauth_providers::UPLOAD,
                activity_id: "1001",
                origin: StreamOrigin::Push,
                source_ref: Some("sha"),
                streams: &samples,
                synced_at: Utc::now(),
            })
            .await
            .expect("store samples");
        assert!(written, "bug: the first write of a row lands");
        let provider = bound(seeded.stores);
        assert_eq!(
            provider.get_activity_streams("1001").await.expect("read"),
            None,
            "bug: another provider key's samples are not Garmin's"
        );
        assert_eq!(
            stores
                .stored_activity_streams(oauth_providers::UPLOAD, "1001")
                .await
                .expect("read"),
            Some(samples),
            "bug: the upload's samples stay under its own key"
        );
    }

    #[tokio::test]
    async fn stats_sum_the_pushed_activities_and_say_so() {
        let seeded = seeded().await;
        let provider = bound(seeded.stores);
        let stats = provider.get_stats().await.expect("stats");

        assert_eq!(stats.total_activities, 2, "bug: both pushed activities");
        assert!(
            (stats.total_distance - 60_000.0).abs() < f64::EPSILON,
            "bug: distance sum {}",
            stats.total_distance
        );
        assert_eq!(stats.total_duration, 7_200, "bug: duration sum");
        assert!(
            (stats.total_elevation_gain - 500.0).abs() < f64::EPSILON,
            "bug: elevation sum {}",
            stats.total_elevation_gain
        );
        match stats.coverage {
            StatsCoverage::RecentActivities { count, oldest } => {
                assert_eq!(count, 2, "bug: coverage counts the pushed rows");
                assert_eq!(
                    oldest.map(|at| at.timestamp()),
                    Some(seeded.ride_start.timestamp()),
                    "bug: coverage starts at the oldest pushed row"
                );
            }
            other => panic!("pushed totals are not a lifetime: {other:?}"),
        }
    }

    /// Every activity this provider serves carries the source the Garmin
    /// attribution keys on (carnet#1154), as the ingest writes it.
    #[tokio::test]
    async fn every_served_activity_carries_the_garmin_attribution() {
        let provider = bound(seeded().await.stores);
        for activity in provider.get_activities(None, None).await.expect("list") {
            assert_eq!(
                oauth_providers::activity_attribution(activity.source(), activity.device_name())
                    .as_deref(),
                Some(oauth_providers::GARMIN_ATTRIBUTION)
            );
        }
    }
}
