// ABOUTME: get_weather_forecast — forecast weather at coordinates, a named place or the athlete's top place
// ABOUTME: dravr-meteo's schema and answer shape, plus the day's daylight and the default place it used
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use dravr_meteo::{
    GeocodeError, Geocoder, Place, PlaceQuery, WeatherError, WeatherProvider, WeatherQuery,
    WeatherSample,
};
use pierre_core::models::{PlaceRef, TenantId};
use pierre_database::repositories::PlaceNameSource;
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::daylight::{daylight, solar_date, Daylight};
use pierre_fitness_compute::geocoding::free_text_query;
use pierre_providers::ai_scope;
use pierre_services::athlete_clock::athlete_zone;
use uuid::Uuid;

use crate::activity_detail_store::served_place_name;
use crate::context::ToolExecutionContext;

use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

/// Longest place name accepted, in characters — dravr-meteo's bound; a longer
/// one is not a place name.
const MAX_PLACE_CHARS: usize = 200;

/// What `get_weather_forecast` answers with.
///
/// The query as resolved and the provider's sample — dravr-meteo's shape for
/// the tool of the same name — plus `daylight` and `default_place`, which the
/// platform adds (carnet#1486).
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
    /// Sunrise and sunset of the place's day, on the athlete's clock.
    pub daylight: ForecastDaylight,
    /// The athlete's training place the forecast was read at, when the
    /// caller gave no location. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_place: Option<DefaultPlace>,
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

/// Sunrise and sunset of the forecast place's solar day.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ForecastDaylight {
    /// RFC 3339 sunrise on the athlete's clock. Absent on a polar day or
    /// night.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunrise: Option<String>,
    /// RFC 3339 sunset on the athlete's clock. Absent on a polar day or
    /// night.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunset: Option<String>,
    /// `day` when the Sun never sets that day, `night` when it never rises.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polar: Option<String>,
}

/// The athlete's most frequent training place, used when no location was
/// given.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DefaultPlace {
    /// The place's id.
    pub id: String,
    /// The place's name, when it has one Dravr may serve here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A training place a forecast may be read at: the coordinates sent, already
/// rounded, and how the answer names the place.
#[derive(Debug, Clone)]
pub struct TrainingPlace {
    /// Latitude sent to the vendor, rounded to 0.01°.
    pub latitude: f64,
    /// Longitude sent to the vendor, rounded to 0.01°.
    pub longitude: f64,
    /// The place as the answer names it.
    pub place: DefaultPlace,
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

/// Tool returning the weather forecast at a place and hour.
///
/// The one `get_weather_forecast` definition: dravr-meteo's (a timestamp, and
/// either latitude and longitude or a place name to geocode, optionally
/// narrowed by `country_code`). The platform's other weather path
/// (`get_weather_for_activity`) resolves weather for an already-recorded
/// activity; this one forecasts a planned one.
#[derive(Debug)]
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
                description: Some("RFC 3339 instant within the next ~16 days".to_owned()),
                format: Some("date-time".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "place".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Place name instead of coordinates. A region and country written out after \
                     it (\"Paris, Texas, United States\") choose among same-named places; an \
                     abbreviation chooses nothing. Ignored when latitude and longitude are given"
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
            "Forecast weather (temperature, humidity, wind, conditions) for the hour of a \
             timestamp up to about 16 days ahead. Call it for every outdoor session you plan, \
             a route included, at the session's hour. Give the location as \
             latitude and longitude, or as a place name (optionally narrowed by country_code). \
             For a place, the output names the town used and says when an ambiguous name took \
             the top-ranked match. Omit the location for the athlete's most frequent training \
             place (Dravr's own app and chats only). The output carries that day's sunrise and \
             sunset, on the athlete's clock.",
            schema,
            // Hits the Open-Meteo forecast API (external infrastructure).
            annotations("Get weather forecast", Effect::ReadOnly, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Auth-gated read tool: it's an agent feature used in authenticated chat,
        // so it requires a valid bearer token to both discover and call.
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result = async {
            // The context's forecast provider and its one place geocoder: a
            // place resolves here exactly as it does for discover_routes and
            // the weather backfill (carnet#920).
            let weather = state.weather();
            let zone = athlete_zone(state.repos(), context.user_id).await;
            if let Some(place) = default_for(&context, &args).await? {
                return forecast_at_place(weather.forecast()?, &args, zone, place).await;
            }
            forecast(weather.forecast()?, weather.geocoder()?, &args, zone).await
        };
        tool_result_to_response(result.await)
    }
}

/// The athlete's training place a forecast with no location is read at.
///
/// `None` when a location was given, or where whose place to use is
/// ambiguous or not Dravr's to send — an external transport, a coach's read
/// of an athlete, a turn other people read — or when the athlete has none.
async fn default_for(
    context: &ToolExecutionContext,
    args: &Value,
) -> AppResult<Option<TrainingPlace>> {
    let located = ["latitude", "longitude", "place"]
        .iter()
        .any(|name| given(args, name));
    if located
        || !may_use_athlete_places(
            ai_scope::withholds_athlete_only_fields(),
            context.turn_direct_message,
        )
    {
        return Ok(None);
    }
    let Some(tenant) = context.tenant_id else {
        return Ok(None);
    };
    default_place(
        context.resources.repos(),
        TenantId::from_uuid(tenant),
        context.user_id,
    )
    .await
}

/// Whether a read may use the athlete's places: a forecast with no location
/// defaults to their place, and a route search offers their own loops.
///
/// Not on an external transport nor a coach's read (`withholds_athlete_only`
/// — the centroid is derived from provider data, the same reason automatic
/// place names are first-party only), and not on a turn other people read
/// (`turn_direct_message == Some(false)`), where whose place is meant is
/// ambiguous.
pub(super) const fn may_use_athlete_places(
    withholds_athlete_only: bool,
    turn_direct_message: Option<bool>,
) -> bool {
    !withholds_athlete_only && !matches!(turn_direct_message, Some(false))
}

/// The athlete's most frequent training place, its centroid rounded to 0.01°.
///
/// Most workouts wins, ties on the lowest id. A 0.01° cell is about a
/// kilometre, so the weather vendor learns a cell rather than the point
/// (carnet#1486). `None` when the athlete has no place.
///
/// # Errors
///
/// Returns the repository error when the places cannot be read.
pub async fn default_place(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Option<TrainingPlace>> {
    let places = repos.athlete_places.list(&tenant_id, user_id).await?;
    let top = places.into_iter().min_by(|a, b| {
        b.activity_count
            .cmp(&a.activity_count)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(top.map(|top| {
        let athlete_named = top.name_source == Some(PlaceNameSource::Athlete);
        let name = served_place_name(&PlaceRef {
            id: top.id.clone(),
            name: top.name,
            athlete_named,
        });
        TrainingPlace {
            latitude: round2(top.centroid_lat),
            longitude: round2(top.centroid_lon),
            place: DefaultPlace { id: top.id, name },
        }
    }))
}

/// `degrees` rounded to 0.01°.
fn round2(degrees: f64) -> f64 {
    (degrees * 100.0).round() / 100.0
}

/// Forecast at the athlete's training `place`, for the `timestamp` in `args`,
/// with the day's daylight on `zone`.
///
/// # Errors
///
/// Only when the result fails to serialize.
pub async fn forecast_at_place(
    provider: &dyn WeatherProvider,
    args: &Value,
    zone: Tz,
    place: TrainingPlace,
) -> AppResult<ToolResult> {
    let timestamp = match parse_timestamp(args) {
        Ok(timestamp) => timestamp,
        Err(message) => return Ok(refusal(&message)),
    };
    let query = WeatherQuery {
        latitude: place.latitude,
        longitude: place.longitude,
        timestamp,
    };
    let note =
        Some("no location given: used the athlete's most frequent training place".to_owned());
    fetch(
        provider,
        query,
        Found {
            place: None,
            note,
            default_place: Some(place.place),
        },
        zone,
    )
    .await
}

/// Run one forecast query against `provider` and render its sample.
///
/// The location is coordinates, or a place name `geocoder` resolves first;
/// the day's daylight is printed on `zone`, the athlete's clock. Anything else
/// is the error an agent can act on.
///
/// # Errors
///
/// Only when the result fails to serialize; a bad argument, an unknown place or
/// a vendor failure is a [`ToolResult::error`] the agent reads.
pub async fn forecast(
    provider: &dyn WeatherProvider,
    geocoder: &dyn Geocoder,
    args: &Value,
    zone: Tz,
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
            fetch(
                provider,
                query,
                Found {
                    place: None,
                    note,
                    default_place: None,
                },
                zone,
            )
            .await
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
                fetch(
                    provider,
                    query,
                    Found {
                        place: Some(answer),
                        note,
                        default_place: None,
                    },
                    zone,
                )
                .await
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

/// How a forecast's location was found: the place a name resolved to, the
/// training place used for none, and why the answer may not be the place
/// meant.
struct Found {
    place: Option<ForecastPlace>,
    note: Option<String>,
    default_place: Option<DefaultPlace>,
}

/// Look the query up and render the sample with its day's daylight on
/// `zone`, or the error.
async fn fetch(
    provider: &dyn WeatherProvider,
    query: WeatherQuery,
    found: Found,
    zone: Tz,
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
                place: found.place,
                latitude: query.latitude,
                longitude: query.longitude,
                timestamp: query.timestamp.to_rfc3339(),
                sample: sample.into(),
                daylight: forecast_daylight(&query, zone),
                default_place: found.default_place,
                note: found.note,
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

/// Sunrise and sunset of the place's solar day at the instant `query` reads,
/// computed at the coordinates it reads, printed on `zone`.
fn forecast_daylight(query: &WeatherQuery, zone: Tz) -> ForecastDaylight {
    let day = solar_date(query.timestamp, query.longitude);
    let on_clock = |instant: DateTime<Utc>| Some(instant.with_timezone(&zone).to_rfc3339());
    match daylight(query.latitude, query.longitude, day) {
        Daylight::Day { sunrise, sunset } => ForecastDaylight {
            sunrise: on_clock(sunrise),
            sunset: on_clock(sunset),
            polar: None,
        },
        Daylight::PolarDay => ForecastDaylight {
            sunrise: None,
            sunset: None,
            polar: Some("day".to_owned()),
        },
        Daylight::PolarNight => ForecastDaylight {
            sunrise: None,
            sunset: None,
            polar: Some("night".to_owned()),
        },
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
        .map_or_default(str::trim);
    if name.is_empty() || name.chars().count() > MAX_PLACE_CHARS {
        return Err(format!(
            "place must be a place name of 1 to {MAX_PLACE_CHARS} characters"
        ));
    }
    let country_code = if given(args, "country_code") {
        let code = args
            .get("country_code")
            .and_then(Value::as_str)
            .map_or_default(str::trim);
        if code.len() != 2 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(format!(
                "country_code must be an ISO 3166-1 alpha-2 code such as \"FR\", got \"{code}\""
            ));
        }
        Some(code.to_ascii_uppercase())
    } else {
        None
    };
    // The gazetteer searches names alone; a region or country appended after a
    // comma chooses among same-named places rather than failing the lookup.
    free_text_query(name, country_code)
        .map(Location::Place)
        .ok_or_else(|| format!("place must be a place name of 1 to {MAX_PLACE_CHARS} characters"))
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
crate::declare_security!(GetWeatherForecastTool => empty);

#[cfg(test)]
mod tests {
    use chrono_tz::America::Montreal;

    use super::*;

    #[test]
    fn a_centroid_is_rounded_to_the_hundredth_of_a_degree() {
        assert!(
            (round2(45.507_49) - 45.51).abs() < 1e-9,
            "bug: 45.50749 rounds to 45.51"
        );
        assert!(
            (round2(-73.572_51) + 73.57).abs() < 1e-9,
            "bug: -73.57251 rounds to -73.57"
        );
    }

    #[test]
    fn only_dravrs_own_one_to_one_surfaces_default_to_the_place() {
        assert!(
            may_use_athlete_places(false, Some(true)),
            "bug: a direct chat turn defaults"
        );
        assert!(
            may_use_athlete_places(false, None),
            "bug: an app call outside a turn defaults"
        );
        assert!(
            !may_use_athlete_places(false, Some(false)),
            "bug: a turn other people read must not default"
        );
        assert!(
            !may_use_athlete_places(true, Some(true)),
            "bug: an external transport or a coach's read must not default"
        );
    }

    #[test]
    fn daylight_is_printed_on_the_athletes_clock() {
        let query = WeatherQuery {
            latitude: 45.50,
            longitude: -73.57,
            timestamp: DateTime::parse_from_rfc3339("2026-06-21T16:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let shown = forecast_daylight(&query, Montreal);
        let sunset = shown.sunset.expect("bug: Montréal has a sunset in June");
        assert!(
            sunset.starts_with("2026-06-21T20:4") && sunset.ends_with("-04:00"),
            "bug: the sunset must read on the athlete's clock: {sunset}"
        );
        assert_eq!(shown.polar, None, "bug: Montréal has no polar day in June");
    }
}
