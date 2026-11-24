// ABOUTME: The one place the platform builds its weather clients: history, forecast and the place geocoder
// ABOUTME: Each is built at the origin the deployment's weather configuration names, never at one a stored document names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where the platform's weather calls go, and the clients that make them.
//!
//! dravr-meteo reads four endpoints: the Open-Meteo archive, the Open-Meteo
//! forecast, the Open-Meteo gazetteer, and `OpenWeatherMap`'s time machine.
//! A server context builds one [`WeatherClients`] from the `weather_api` of
//! its deployment configuration and every weather call goes through it: a
//! client built where it is used sits on the vendor's own host, where no
//! context can name another (carnet#1615).
//!
//! The configuration is the deployment's, read once at startup. It is never
//! a stored fitness configuration: an athlete can write that document, and an
//! origin read from it would let them aim the server's requests at a host of
//! their choosing.

use std::fmt;
use std::sync::{Arc, OnceLock};

use dravr_meteo::{
    provider_from_env_at, CacheLimits, CachedGeocoder, CachedProvider, ClientBuildError,
    ConfigError, Geocoder, InMemoryStore, OpenMeteoForecastProvider, OpenMeteoGeocoder,
    VendorBaseUrls, WeatherCacheStore, WeatherProvider,
};
use pierre_core::config::fitness::WeatherApiConfig;
use pierre_core::errors::{AppError, AppResult};

/// The path the Open-Meteo archive is read at, under a single origin.
const ARCHIVE_PATH: &str = "/v1/archive";
/// The path the Open-Meteo forecast is read at, under a single origin.
const FORECAST_PATH: &str = "/v1/forecast";
/// The path the Open-Meteo gazetteer is searched at, under a single origin.
const GEOCODING_PATH: &str = "/v1/search";

/// Where each weather endpoint is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WeatherOrigins {
    /// Each vendor on its own host: the addresses dravr-meteo's clients are
    /// built on when they are given none.
    Vendors,
    /// One origin for all four endpoints, each at its vendor's path.
    Single {
        /// The Open-Meteo archive endpoint, `{origin}/v1/archive`.
        archive: String,
        /// The Open-Meteo forecast endpoint, `{origin}/v1/forecast`.
        forecast: String,
        /// The Open-Meteo gazetteer endpoint, `{origin}/v1/search`.
        geocoding: String,
        /// The origin `OpenWeatherMap`'s `/data/3.0/…` path is appended to.
        openweathermap: String,
    },
}

impl WeatherOrigins {
    /// The origins a deployment's `weather_api` names.
    ///
    /// No `weather_api`, or one whose `base_url` is
    /// [`WeatherApiConfig::DEFAULT_BASE_URL`], is the vendors' own hosts. Any
    /// other `base_url` is the single origin of all four endpoints; the
    /// slashes it ends in are dropped before each vendor's path is appended.
    #[must_use]
    fn from_config(weather_api: Option<&WeatherApiConfig>) -> Self {
        let Some(origin) = weather_api.map(|api| api.base_url.trim_end_matches('/')) else {
            return Self::Vendors;
        };
        if origin == WeatherApiConfig::DEFAULT_BASE_URL {
            return Self::Vendors;
        }
        Self::Single {
            archive: format!("{origin}{ARCHIVE_PATH}"),
            forecast: format!("{origin}{FORECAST_PATH}"),
            geocoding: format!("{origin}{GEOCODING_PATH}"),
            openweathermap: origin.to_owned(),
        }
    }

    /// The historical vendors' base URLs, as dravr-meteo takes them:
    /// its own defaults for [`Self::Vendors`].
    #[must_use]
    fn history_base_urls(&self) -> VendorBaseUrls {
        match self {
            Self::Vendors => VendorBaseUrls::default(),
            Self::Single {
                archive,
                openweathermap,
                ..
            } => VendorBaseUrls::default()
                .with_open_meteo_archive(archive.as_str())
                .with_openweathermap(openweathermap.as_str()),
        }
    }
}

/// The forecast provider behind the bounded in-process cache dravr-meteo
/// sizes for forecasts.
type Forecast = CachedProvider<OpenMeteoForecastProvider, InMemoryStore>;

/// A server context's weather clients, built at the origins its deployment
/// configuration names.
///
/// One per context, so the geocoder's and the forecast's caches live as long
/// as the context does and every surface that takes a place resolves it
/// through the same gazetteer and cache (carnet#920). A client is built on
/// first use; one that could not be built is reported to every caller rather
/// than replaced by one without a deadline.
pub struct WeatherClients {
    backfill_enabled: bool,
    origins: WeatherOrigins,
    geocoder: OnceLock<Result<CachedGeocoder<OpenMeteoGeocoder>, ClientBuildError>>,
    forecast: OnceLock<Result<Forecast, ClientBuildError>>,
}

impl fmt::Debug for WeatherClients {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeatherClients")
            .field("backfill_enabled", &self.backfill_enabled)
            .field("origins", &self.origins)
            .finish_non_exhaustive()
    }
}

impl WeatherClients {
    /// The clients for a deployment whose weather configuration is
    /// `weather_api`.
    ///
    /// Hand this the `weather_api` of the configuration the server read at
    /// startup, never the one of a stored fitness configuration: an athlete
    /// writes that document, and its `base_url` would send the server's
    /// requests wherever they chose.
    ///
    /// The activity list's temperature backfill runs when `weather_api` is
    /// present and enabled. Without a `weather_api` there is no backfill, and
    /// the tools an athlete asks for weather with read the vendors' own
    /// hosts.
    #[must_use]
    pub fn new(weather_api: Option<&WeatherApiConfig>) -> Self {
        Self {
            backfill_enabled: weather_api.is_some_and(|api| api.enabled),
            origins: WeatherOrigins::from_config(weather_api),
            geocoder: OnceLock::new(),
            forecast: OnceLock::new(),
        }
    }

    /// Whether an activity list is given the ambient temperature its source
    /// left out.
    #[must_use]
    pub const fn backfill_enabled(&self) -> bool {
        self.backfill_enabled
    }

    /// The historical provider `WEATHER_PROVIDER` selects, behind `cache`.
    ///
    /// # Errors
    ///
    /// dravr-meteo's [`ConfigError`] for an environment that selects no
    /// working vendor.
    pub fn history<S>(&self, cache: S) -> Result<Arc<dyn WeatherProvider>, ConfigError>
    where
        S: WeatherCacheStore + 'static,
    {
        provider_from_env_at(cache, &self.origins.history_base_urls())
    }

    /// The forecast provider every call on this context shares: a stored
    /// sample is reused for an hour, in a bounded store.
    ///
    /// # Errors
    ///
    /// An external-service error when its HTTP client could not be built.
    pub fn forecast(&self) -> AppResult<&dyn WeatherProvider> {
        let built = self.forecast.get_or_init(|| {
            let provider = match &self.origins {
                WeatherOrigins::Vendors => OpenMeteoForecastProvider::new(),
                WeatherOrigins::Single { forecast, .. } => {
                    OpenMeteoForecastProvider::with_base_url(forecast.clone())
                }
            };
            provider.map(|provider| {
                CachedProvider::new(provider, InMemoryStore::with_limits(CacheLimits::FORECAST))
            })
        });
        match built {
            Ok(provider) => Ok(provider),
            Err(e) => Err(AppError::external_service(
                "Open-Meteo forecast",
                e.to_string(),
            )),
        }
    }

    /// The place geocoder every caller on this context shares: the gazetteer
    /// behind the bounded cache dravr-meteo sizes for place lookups. Only
    /// resolutions are cached; an unknown name reaches the gazetteer again.
    ///
    /// # Errors
    ///
    /// An external-service error when its HTTP client could not be built.
    pub fn geocoder(&self) -> AppResult<&dyn Geocoder> {
        let built = self.geocoder.get_or_init(|| {
            let geocoder = match &self.origins {
                WeatherOrigins::Vendors => OpenMeteoGeocoder::new(),
                WeatherOrigins::Single { geocoding, .. } => {
                    OpenMeteoGeocoder::with_base_url(geocoding.clone())
                }
            };
            geocoder.map(|geocoder| CachedGeocoder::new(geocoder, CacheLimits::GEOCODE))
        });
        match built {
            Ok(geocoder) => Ok(geocoder),
            Err(e) => Err(AppError::external_service(
                "Open-Meteo geocoding",
                e.to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather_api(base_url: &str, enabled: bool) -> WeatherApiConfig {
        WeatherApiConfig {
            base_url: base_url.to_owned(),
            enabled,
        }
    }

    #[test]
    fn the_default_base_url_is_the_vendors_own_hosts() {
        let default = WeatherApiConfig::default();
        let origins = WeatherOrigins::from_config(Some(&default));
        // The forecast and gazetteer clients are built with dravr-meteo's own
        // `new`, and the historical ones on its own default base URLs.
        assert_eq!(origins, WeatherOrigins::Vendors);
        assert_eq!(origins.history_base_urls(), VendorBaseUrls::default());

        let slashed = weather_api("https://api.openweathermap.org/", true);
        assert_eq!(
            WeatherOrigins::from_config(Some(&slashed)),
            WeatherOrigins::Vendors
        );
        assert_eq!(WeatherOrigins::from_config(None), WeatherOrigins::Vendors);
    }

    #[test]
    fn any_other_base_url_is_the_origin_of_all_four_endpoints() {
        for written in ["http://127.0.0.1:9", "http://127.0.0.1:9//"] {
            let origins = WeatherOrigins::from_config(Some(&weather_api(written, true)));
            assert_eq!(
                origins,
                WeatherOrigins::Single {
                    archive: "http://127.0.0.1:9/v1/archive".to_owned(),
                    forecast: "http://127.0.0.1:9/v1/forecast".to_owned(),
                    geocoding: "http://127.0.0.1:9/v1/search".to_owned(),
                    openweathermap: "http://127.0.0.1:9".to_owned(),
                }
            );
            assert_eq!(
                origins.history_base_urls(),
                VendorBaseUrls::default()
                    .with_open_meteo_archive("http://127.0.0.1:9/v1/archive")
                    .with_openweathermap("http://127.0.0.1:9")
            );
            assert_ne!(origins.history_base_urls(), VendorBaseUrls::default());
        }
    }

    #[test]
    fn the_backfill_runs_only_on_an_enabled_weather_api() {
        assert!(WeatherClients::new(Some(&WeatherApiConfig::default())).backfill_enabled());
        assert!(
            !WeatherClients::new(Some(&weather_api("http://127.0.0.1:9", false)))
                .backfill_enabled()
        );
        assert!(!WeatherClients::new(None).backfill_enabled());
    }
}
