// ABOUTME: get_weather_forecast — forecast weather at coordinates or a named place, up to ~16 days ahead
// ABOUTME: Takes dravr-meteo's definition of the tool: its schema, answer shape, cached provider and geocoder
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use dravr_meteo::{
    CacheLimits, CachedGeocoder, CachedProvider, GeocodeError, Geocoder, InMemoryStore,
    OpenMeteoForecastProvider, OpenMeteoGeocoder, Place, PlaceQuery, WeatherError, WeatherProvider,
    WeatherQuery, WeatherSample,
};

use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// The forecast provider every call shares, behind the bounded in-process
/// cache dravr-meteo sizes for forecasts ([`CacheLimits::FORECAST`]): a
/// stored sample is reused for an hour, the fastest model run a vendor
/// ingests, and the store holds a bounded number of entries. This is the
/// provider dravr-meteo's own `get_weather_forecast` serves from.
static FORECAST: LazyLock<CachedProvider<OpenMeteoForecastProvider, InMemoryStore>> =
    LazyLock::new(|| {
        CachedProvider::new(
            OpenMeteoForecastProvider::new(),
            InMemoryStore::with_limits(CacheLimits::FORECAST),
        )
    });

/// The place-name geocoder every call shares, built beside [`FORECAST`]:
/// Open-Meteo's gazetteer behind the bounded cache dravr-meteo sizes for
/// place lookups ([`CacheLimits::GEOCODE`]). Only resolutions are cached; an
/// unknown name reaches the gazetteer again next time.
static GEOCODER: LazyLock<CachedGeocoder<OpenMeteoGeocoder>> =
    LazyLock::new(|| CachedGeocoder::new(OpenMeteoGeocoder::new(), CacheLimits::GEOCODE));

/// Longest place name accepted, in characters — dravr-meteo's bound; a longer
/// one is not a place name.
const MAX_PLACE_CHARS: usize = 200;

/// What `get_weather_forecast` answers with.
///
/// The query as resolved and the provider's sample — the shape dravr-meteo's
/// tool of the same name answers with, so the one tool reads the same from
/// either server.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct WeatherForecastResult {
    /// The vendor that produced the sample.
    pub provider: String,
    /// The place a `place` argument resolved to. Absent when the caller gave
    /// coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<ForecastPlace>,
    /// Latitude the forecast was read at.
    pub latitude: f64,
    /// Longitude the forecast was read at.
    pub longitude: f64,
    /// RFC 3339 instant the sample is for.
    pub timestamp: String,
    /// The forecast for the hour containing `timestamp`.
    pub sample: ForecastSample,
    /// Why the answer may not be the place the caller meant: an ambiguous name
    /// took its top-ranked match, or coordinates overrode a `place` given too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The place a name resolved to, serialized exactly as dravr-meteo's tool
/// reports it: the name as typed, then its `Place`, then the match count.
///
/// A schema-bearing view: dravr-meteo does not derive `schemars::JsonSchema`,
/// and the tool declares its output schema.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ForecastPlace {
    /// The name as the caller gave it.
    pub query: String,
    /// The gazetteer's name for the place used.
    pub name: String,
    /// First-level administrative area (region, state, province), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin1: Option<String>,
    /// Country name, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// ISO 3166-1 alpha-2 country code, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    /// Latitude of the place, decimal degrees, north-positive.
    pub latitude: f64,
    /// Longitude of the place, decimal degrees, east-positive.
    pub longitude: f64,
    /// Places the name matched; more than one means the top-ranked was used.
    pub matches: usize,
}

/// One forecast hour, serialized exactly as dravr-meteo's `WeatherSample`.
///
/// A schema-bearing view of that type: dravr-meteo does not derive
/// `schemars::JsonSchema`, and the tool declares its output schema.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ForecastSample {
    /// Air temperature in degrees Celsius.
    pub temperature_celsius: f32,
    /// Relative humidity, 0–100. Absent when the vendor did not report it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humidity_percentage: Option<f32>,
    /// Wind speed in km/h. Absent when the vendor did not report it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_speed_kmh: Option<f32>,
    /// Sky and precipitation, in words.
    pub conditions: String,
}

impl From<WeatherSample> for ForecastSample {
    fn from(sample: WeatherSample) -> Self {
        Self {
            temperature_celsius: sample.temperature_celsius,
            humidity_percentage: sample.humidity_percentage,
            wind_speed_kmh: sample.wind_speed_kmh,
            conditions: sample.conditions,
        }
    }
}

fn forecast_annotations() -> ToolAnnotations {
    ToolAnnotations {
        title: Some("Weather forecast".to_owned()),
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        // Hits the Open-Meteo forecast API (external infrastructure).
        open_world_hint: Some(true),
    }
}

/// Tool returning the weather forecast at a place and hour.
///
/// The one `get_weather_forecast` definition: dravr-meteo's (a timestamp, and
/// either latitude and longitude or a place name to geocode, optionally
/// narrowed by `country_code`). The platform's other weather path
/// (`get_weather_for_activity`) resolves weather for an already-recorded
/// activity; this one forecasts a planned one.
pub struct GetWeatherForecastTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetWeatherForecastTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "latitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Decimal degrees, north-positive. Give with longitude, or give place instead"
                        .to_owned(),
                ),
                minimum: Some(-90.0),
                maximum: Some(90.0),
                ..Default::default()
            },
        );
        properties.insert(
            "longitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Decimal degrees, east-positive. Give with latitude, or give place instead"
                        .to_owned(),
                ),
                minimum: Some(-180.0),
                maximum: Some(180.0),
                ..Default::default()
            },
        );
        properties.insert(
            "timestamp".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Instant within the next ~16 days, RFC 3339; looked up at the hour it falls in"
                        .to_owned(),
                ),
                format: Some("date-time".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "place".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Place name to geocode instead of coordinates, e.g. \"Chamonix\" — the name \
                     only, no region or country appended. Ignored when latitude and longitude \
                     are given"
                        .to_owned(),
                ),
                min_length: Some(1),
                max_length: Some(MAX_PLACE_CHARS as u64),
                ..Default::default()
            },
        );
        properties.insert(
            "country_code".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "ISO 3166-1 alpha-2 country narrowing place, e.g. \"FR\"".to_owned(),
                ),
                pattern: Some("^[A-Za-z]{2}$".to_owned()),
                ..Default::default()
            },
        );

        // Which of coordinates or place is given is checked at run time, not
        // by the schema: a top-level `oneOf`/`anyOf` is refused by several LLM
        // function-calling APIs, which would drop the tool for every agent
        // behind one of them.
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties.into_iter().collect()),
            required: Some(vec!["timestamp".to_owned()]),
            ..Default::default()
        };
        answers_with::<WeatherForecastResult>(tool_definition(
            "get_weather_forecast",
            "Forecast weather — temperature, humidity, wind and conditions — for the hour \
             containing a timestamp up to about 16 days ahead, e.g. a planned session. Give \
             the location either as latitude and longitude, or as a place name (optionally \
             narrowed by country_code), which is geocoded; when both are given the \
             coordinates win and place is ignored. For a place, the output names the town \
             used, and an ambiguous name takes the top-ranked match and says so.",
            schema,
            Some(forecast_annotations()),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Auth-gated read tool: it's an agent feature used in authenticated chat,
        // so it requires a valid bearer token to both discover and call.
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        tool_result_to_response(forecast(&*FORECAST, &*GEOCODER, &args).await)
    }
}

/// Run one forecast query — by coordinates, or by a place name `geocoder`
/// resolves first — against `provider`, and render the sample, or the error an
/// agent can act on.
///
/// # Errors
///
/// Only when the result fails to serialize; a bad argument, an unknown place or
/// a vendor failure is a [`ToolResult::error`] the agent reads.
pub async fn forecast(
    provider: &dyn WeatherProvider,
    geocoder: &dyn Geocoder,
    args: &Value,
) -> AppResult<ToolResult> {
    let location = match parse_location(args) {
        Ok(location) => location,
        Err(message) => return Ok(refusal(&message)),
    };
    let timestamp = match parse_timestamp(args) {
        Ok(timestamp) => timestamp,
        Err(message) => return Ok(refusal(&message)),
    };
    match location {
        Location::Coordinates {
            latitude,
            longitude,
            place_ignored,
        } => {
            let note = place_ignored.then(|| {
                "place was ignored: latitude and longitude were given, and coordinates win"
                    .to_owned()
            });
            let query = WeatherQuery {
                latitude,
                longitude,
                timestamp,
            };
            fetch(provider, query, None, note).await
        }
        Location::Place(place) => match geocoder.resolve(&place).await {
            Ok(resolution) => {
                let note = (resolution.matches > 1).then(|| {
                    format!(
                        "\"{}\" matched {} places; used the top-ranked one, {}. Pass country_code, or latitude and longitude, for a different one",
                        place.name,
                        resolution.matches,
                        describe(&resolution.place)
                    )
                });
                let query = WeatherQuery {
                    latitude: resolution.place.latitude,
                    longitude: resolution.place.longitude,
                    timestamp,
                };
                let resolved = resolution.place;
                let answer = ForecastPlace {
                    query: place.name,
                    name: resolved.name,
                    admin1: resolved.admin1,
                    country: resolved.country,
                    country_code: resolved.country_code,
                    latitude: resolved.latitude,
                    longitude: resolved.longitude,
                    matches: resolution.matches,
                };
                fetch(provider, query, Some(answer), note).await
            }
            Err(GeocodeError::NotFound { place }) => Ok(refusal(&format!(
                "no place named \"{place}\" was found — check the spelling, give the name alone (a country goes in country_code), or pass latitude and longitude"
            ))),
            Err(err) => {
                warn!(error = %err, "get_weather_forecast: place lookup failed");
                Ok(refusal(&format!(
                    "could not look up place \"{}\": {err}",
                    place.name
                )))
            }
        },
    }
}

/// The error an agent reads, in the shape every refusal of this tool takes.
fn refusal(message: &str) -> ToolResult {
    ToolResult::error(json!({ "error": message }))
}

/// Look the query up and render the sample, or the error.
async fn fetch(
    provider: &dyn WeatherProvider,
    query: WeatherQuery,
    place: Option<ForecastPlace>,
    note: Option<String>,
) -> AppResult<ToolResult> {
    info!(
        latitude = query.latitude,
        longitude = query.longitude,
        timestamp = %query.timestamp,
        "get_weather_forecast: querying the forecast"
    );
    match provider.weather_at(query).await {
        Ok(sample) => ok_typed(
            "get_weather_forecast",
            WeatherForecastResult {
                provider: provider.name().to_owned(),
                place,
                latitude: query.latitude,
                longitude: query.longitude,
                timestamp: query.timestamp.to_rfc3339(),
                sample: sample.into(),
                note,
            },
        ),
        Err(WeatherError::DataUnavailable) => Ok(refusal(&format!(
            "{} has no data for that place and hour — the archive does not cover the future, and the forecast covers ~16 days ahead",
            provider.name()
        ))),
        Err(err) => {
            warn!(error = %err, "get_weather_forecast: forecast lookup failed");
            Ok(refusal(&err.to_string()))
        }
    }
}

/// "Name, Region, Country" for a note, skipping the parts the gazetteer lacks.
fn describe(place: &Place) -> String {
    [
        Some(place.name.as_str()),
        place.admin1.as_deref(),
        place.country.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

/// Where the forecast is read.
enum Location {
    /// Coordinates as given; `place_ignored` when a place was given too.
    Coordinates {
        latitude: f64,
        longitude: f64,
        place_ignored: bool,
    },
    /// A place name to geocode first.
    Place(PlaceQuery),
}

/// True when `name` is present and not JSON `null`.
fn given(args: &Value, name: &str) -> bool {
    args.get(name).is_some_and(|value| !value.is_null())
}

/// Decide between coordinates and a place name, and validate whichever is
/// used, with dravr-meteo's rules and messages.
///
/// Coordinates win when both are given; one coordinate without the other is
/// refused rather than completed from `place`.
fn parse_location(args: &Value) -> Result<Location, String> {
    let has_place = given(args, "place");
    if given(args, "latitude") || given(args, "longitude") {
        let (latitude, longitude) = parse_coordinates(args)?;
        return Ok(Location::Coordinates {
            latitude,
            longitude,
            place_ignored: has_place,
        });
    }
    if !has_place {
        return Err(
            "give either latitude and longitude, or place (a place name such as \"Chamonix\")"
                .to_owned(),
        );
    }
    let name = args
        .get("place")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if name.is_empty() || name.chars().count() > MAX_PLACE_CHARS {
        return Err(format!(
            "place must be a place name of 1 to {MAX_PLACE_CHARS} characters"
        ));
    }
    let country_code = if given(args, "country_code") {
        let code = args
            .get("country_code")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if code.len() != 2 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(format!(
                "country_code must be an ISO 3166-1 alpha-2 code such as \"FR\", got \"{code}\""
            ));
        }
        Some(code.to_ascii_uppercase())
    } else {
        None
    };
    Ok(Location::Place(PlaceQuery {
        name: name.to_owned(),
        country_code,
    }))
}

/// Parse and range-check `latitude` and `longitude`.
fn parse_coordinates(args: &Value) -> Result<(f64, f64), String> {
    let coordinate = |name: &str, bound: f64| -> Result<f64, String> {
        let value = args
            .get(name)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("missing required parameter: {name} (a number)"))?;
        if value.is_finite() && value.abs() <= bound {
            Ok(value)
        } else {
            Err(format!(
                "{name} must be between -{bound} and {bound}, got {value}"
            ))
        }
    };
    Ok((
        coordinate("latitude", 90.0)?,
        coordinate("longitude", 180.0)?,
    ))
}

/// Parse the RFC 3339 `timestamp`.
fn parse_timestamp(args: &Value) -> Result<DateTime<Utc>, String> {
    let raw = args
        .get("timestamp")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing required parameter: timestamp (RFC 3339)".to_owned())?;
    Ok(DateTime::parse_from_rfc3339(raw)
        .map_err(|e| format!("timestamp must be RFC 3339, e.g. 2026-09-22T14:00:00Z: {e}"))?
        .with_timezone(&Utc))
}

/// Create weather-forecast tools for registration.
#[must_use]
pub fn create_weather_forecast_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(GetWeatherForecastTool)]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(GetWeatherForecastTool => UNTRUSTED_OUTPUT);
