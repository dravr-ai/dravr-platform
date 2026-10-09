// ABOUTME: Intervals.icu provider — FitnessProvider for athlete profile + activities + streams + wellness + the training-calendar write surface
// ABOUTME: Authenticates with the athlete's OAuth bearer token or HTTP Basic API key; calendar writes create, update, and delete events keyed by Dravr's external_id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Clippy allowances for this module:
// - cast_possible_truncation: Intervals.icu returns f64 for HR/power/cadence; truncating to u32 is safe within sensor ranges
// - cast_sign_loss: same — HR/power/cadence are always non-negative
// - cast_precision_loss: same — single-second timestamps fit in f64 without loss
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

//! # Intervals.icu Provider Module
//!
//! [`FitnessProvider`] for Intervals.icu (athlete profile, activities, streams,
//! wellness, and the training-calendar write surface), authenticated with the
//! athlete's OAuth grant or their personal API key.
//!
//! ## Calendar writes
//!
//! A [`PlannedSession`] becomes one calendar event: `POST /events` creates it,
//! `PUT /events/{id}` replaces it in place, `PUT /events/bulk-delete` removes
//! a batch by id, and `GET /events` reads the window back so the ledger can be
//! reconciled against the calendar. Every event carries Dravr's `external_id`,
//! and a session's steps go out in Intervals.icu's workout text DSL (see
//! [`render_description`]) so the calendar parses them into targets and
//! computes planned load. The DSL is parsed on every write and cannot be
//! disabled, which is why agent prose is escaped before it is sent.
//!
//! ## Authentication
//!
//! An athlete links in one of two ways, and [`OAuth2Credentials::kind`] says
//! which a credential is:
//!
//! - **OAuth** ([`CredentialKind::OAuthBearer`]): the athlete authorizes the
//!   Dravr app and the token goes out as `Authorization: Bearer`. Tokens do not
//!   expire and there is no refresh grant; a new authorization replaces the
//!   token. Athlete-scoped paths use the id `0`, which Intervals.icu resolves to
//!   the athlete the token was issued for.
//! - **API key** ([`CredentialKind::ApiKey`]): the athlete pastes the personal
//!   key from their settings. The platform stores the athlete id in
//!   `user_oauth_tokens.provider_user_id` and the key in the encrypted
//!   access-token column, and the serving path feeds them in as `client_id` /
//!   `access_token`. The athlete id addresses the athlete-scoped URL path
//!   (`/api/v1/athlete/{id}/...`), while the Basic credential pair is always
//!   `API_KEY:<api key>`: Intervals.icu rejects an athlete id in the username
//!   position with 401 on every endpoint.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use serde_json::json;

use super::core::{
    no_recorded_samples, ActivityQueryParams, CredentialKind, FitnessProvider, OAuth2Credentials,
    ProviderConfig, ProviderFactory, TokenRefreshCallback,
};
use crate::activity_paging::pages_for;
use crate::constants::api_provider_limits;
use crate::constants::oauth::INTERVALS_ICU;
use crate::delegation::{
    athlete_off_roster, coach_credential_expired, CoachRoster, DelegatedReads,
};
use crate::errors::{AppError, AppResult};
use crate::http_client::{shared_client, SharedHttpClient, SharedHttpError, SharedRequestBuilder};
use crate::intervals_icu_calendar::{
    event_body, event_id_segment, CreatedEvent, DeleteEventsResponse, IntervalsIcuEvent,
};
use crate::intervals_icu_roster::{check_athlete_id, coach_roster, IntervalsIcuRosterEntry};
use crate::intervals_icu_self_report::{
    comments_from_messages, IntervalsIcuMessage, MAX_ACTIVITY_MESSAGES,
};
use crate::intervals_icu_streams::{streams_to_time_series, IntervalsIcuStream};
use crate::models::{
    Activity, ActivityComment, Athlete, CalendarEventRef, PlannedSession, Stats, TimeSeriesData,
};
use crate::pagination::{CursorPage, PaginationParams};
use crate::request_budget;
use crate::spi::{IntervalsIcuDescriptor, ProviderDescriptor};
use crate::utils::auth_error_for_status;

/// The activity payload and its mapping onto [`Activity`].
mod activity;

pub(crate) use activity::parse_local_dt;
use activity::{map_activity, IntervalsIcuActivity};

/// Default base URL for Intervals.icu's REST API (overridable by tests).
pub const DEFAULT_API_BASE_URL: &str = "https://intervals.icu";

/// Where an athlete authorizes the Dravr OAuth app.
pub const OAUTH_AUTHORIZE_URL: &str = "https://intervals.icu/oauth/authorize";

/// Where an authorization code is exchanged for a token.
pub const OAUTH_TOKEN_URL: &str = "https://intervals.icu/api/oauth/token";

/// Where a bearer token deauthorizes itself (`DELETE`).
pub const OAUTH_REVOKE_URL: &str = "https://intervals.icu/api/v1/disconnect-app";

/// The scopes the Dravr app asks for.
pub use crate::constants::oauth::INTERVALS_ICU_DEFAULT_SCOPES as DEFAULT_SCOPES;

/// HTTP Basic auth username for Intervals.icu's API-key scheme.
///
/// Intervals.icu authenticates personal API keys with the *literal* string
/// `API_KEY` as the Basic username and the athlete's key as the password —
/// the athlete id belongs in the URL path, never in the credential pair.
/// Sending the athlete id as the username makes every call 401.
pub const BASIC_AUTH_USERNAME: &str = "API_KEY";

/// Local aliases for the centralised page sizes — every provider's live in
/// `api_provider_limits` so a walk's arithmetic reads one source.
const DEFAULT_PAGE_LIMIT: usize = api_provider_limits::intervals_icu::DEFAULT_ACTIVITIES_PER_PAGE;
const MAX_PAGE_LIMIT: usize = api_provider_limits::intervals_icu::MAX_ACTIVITIES_PER_REQUEST;

/// `strftime` format for the `oldest` / `newest` bounds on a range query.
///
/// Intervals.icu parses these as *local* date-times: no UTC offset, no
/// fractional seconds. `to_rfc3339()` produces both, and the API answers a
/// request carrying them with 422 — which is what every activity list call
/// against the live service got (prod, 2026-08-26: `GET
/// /api/v1/athlete/{id}/activities` → 422). The same file already speaks this
/// dialect everywhere else: `/wellness` and `/events` send `%Y-%m-%d`, and
/// [`parse_local_dt`] reads responses back with this exact pattern.
const QUERY_DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// How far back an activity list reaches when the caller names no lower bound.
///
/// Intervals.icu requires a bounded range — an unbounded list is a 422, not a
/// full-history dump — so every entry point defaults rather than passing the
/// absence through.
const DEFAULT_LOOKBACK_DAYS: i64 = 90;

/// How far forward an activity list reaches when the caller names no upper
/// bound. One day, so an activity uploaded earlier today is inside the window.
const DEFAULT_LOOKAHEAD_DAYS: i64 = 1;

/// Slack subtracted from the lower bound before it goes on the wire, absorbing
/// the local-vs-UTC reading of [`QUERY_DATETIME_FORMAT`].
///
/// What we hold is a UTC instant; what the format puts on the wire is a naive
/// wall clock, and Intervals.icu reads that as *athlete-local*. For an athlete
/// west of UTC our wall clock runs ahead of theirs, so an unpadded `oldest` is
/// read as up to twelve hours later than the caller asked for and activities
/// inside that gap never come back. The upper bound already carries
/// [`DEFAULT_LOOKAHEAD_DAYS`] of slack for a different reason; this is its
/// counterpart on the lower bound.
///
/// Widening is the safe direction. Callers dedupe by activity id on the way
/// into the cache (`write_through_activity_cache`), so an extra day of overlap
/// costs a few redundant rows, while a missing one costs an activity the
/// athlete uploaded this morning — the incremental `after` fetches
/// (`fetch_recent_activities_all_providers`, the fresh-head data path) pass a
/// real lower bound and are exactly the callers that would lose it.
const QUERY_LOCAL_OFFSET_SLACK_DAYS: i64 = 1;

/// Resolve the `(oldest, newest)` bounds an activity list query runs with.
///
/// One source of truth for the defaulting, because the two entry points
/// disagreed: [`FitnessProvider::get_activities_with_params`] defaulted both
/// bounds while `get_activities_cursor` passed `None, None` straight through
/// and asked Intervals.icu for an unbounded range.
fn activity_window(
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    (
        after.unwrap_or_else(|| now - Duration::days(DEFAULT_LOOKBACK_DAYS)),
        before.unwrap_or_else(|| now + Duration::days(DEFAULT_LOOKAHEAD_DAYS)),
    )
}

/// Wrap a `reqwest` request with structured before/after tracing so every
/// Intervals.icu API call surfaces in Cloud Run logs with op + url +
/// HTTP status. Credentials never enter the log line — `RequestBuilder`
/// owns the `basic_auth` header and we only ever read the URL we built.
async fn send_traced(
    req: SharedRequestBuilder,
    op: &'static str,
    url: &str,
) -> Result<Response, SharedHttpError> {
    info!(provider = "intervals_icu", op, url, "request");
    match req.send().await {
        Ok(response) => {
            info!(
                provider = "intervals_icu",
                op,
                url,
                status = response.status().as_u16(),
                content_length = response.content_length().unwrap_or(0),
                "response"
            );
            Ok(response)
        }
        Err(e) => {
            warn!(provider = "intervals_icu", op, url, error = %e, "transport failure");
            Err(e)
        }
    }
}

/// The athlete-id path segment Intervals.icu resolves to the athlete an OAuth
/// bearer token was issued for.
const TOKEN_ATHLETE_SEGMENT: &str = "0";

/// How one call authenticates: the athlete's OAuth grant, or their pasted
/// personal API key.
enum CallAuth {
    /// `Authorization: Bearer <token>`; the token names its athlete.
    Bearer(String),
    /// HTTP Basic `API_KEY:<api key>`; the athlete id addresses the path.
    ApiKey { athlete_id: String, api_key: String },
}

impl CallAuth {
    /// The athlete-id segment of an athlete-scoped URL. A bearer token is
    /// addressed as [`TOKEN_ATHLETE_SEGMENT`], never by an id read elsewhere,
    /// so a call can only reach the athlete who granted it.
    fn athlete_segment(&self) -> &str {
        match self {
            Self::Bearer(_) => TOKEN_ATHLETE_SEGMENT,
            Self::ApiKey { athlete_id, .. } => athlete_id,
        }
    }

    fn authorize(&self, req: SharedRequestBuilder) -> SharedRequestBuilder {
        match self {
            Self::Bearer(token) => req.bearer_auth(token),
            Self::ApiKey { api_key, .. } => req.basic_auth(BASIC_AUTH_USERNAME, Some(api_key)),
        }
    }
}

/// The error a call Intervals.icu answered with a non-success `status` is.
///
/// A 401 means the credential no longer works (the athlete withdrew the grant
/// or regenerated the key), which is the reconnect path every provider shares;
/// anything else is the upstream failing.
fn status_error(op: &str, status: StatusCode) -> AppError {
    auth_error_for_status(status, INTERVALS_ICU).unwrap_or_else(|| {
        AppError::external_service(INTERVALS_ICU, format!("{op} returned {status}"))
    })
}

/// What a call names: the athlete a URL path addresses, or one activity by
/// id. A delegated provider reads a refusal of each differently.
#[derive(Clone, Copy)]
enum Target {
    /// `/api/v1/athlete/{id}/...`
    Athlete,
    /// `/api/v1/activity/{id}/...`
    Activity,
}

/// The platform as an athlete reads its name.
fn brand() -> &'static str {
    IntervalsIcuDescriptor.display_name()
}

/// Provider configuration for Intervals.icu: its OAuth endpoints, for athletes
/// who link through the Dravr app, and the API base both link kinds call.
#[must_use]
pub fn default_config() -> ProviderConfig {
    ProviderConfig {
        name: INTERVALS_ICU.to_owned(),
        auth_url: OAUTH_AUTHORIZE_URL.to_owned(),
        token_url: OAUTH_TOKEN_URL.to_owned(),
        api_base_url: DEFAULT_API_BASE_URL.to_owned(),
        revoke_url: Some(OAUTH_REVOKE_URL.to_owned()),
        default_scopes: DEFAULT_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// Intervals.icu athlete profile shape.
#[derive(Debug, Deserialize)]
struct IntervalsIcuAthlete {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    profile_medium: Option<String>,
}

/// Provider implementation for Intervals.icu.
pub struct IntervalsIcuProvider {
    config: ProviderConfig,
    /// Stored credentials. `access_token` holds the bearer token or the API
    /// key, as `kind` says. For an API key, `client_id` holds the athlete id,
    /// which addresses the athlete-scoped URL path; the HTTP Basic username is
    /// the constant [`BASIC_AUTH_USERNAME`], never the athlete id.
    credentials: Arc<RwLock<Option<OAuth2Credentials>>>,
    http: SharedHttpClient,
    /// The coached athlete a delegated provider reads through a coach's
    /// credential, by Intervals.icu athlete id; `None` reads the credential's
    /// own athlete.
    subject: Option<String>,
}

impl IntervalsIcuProvider {
    /// Construct with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(default_config())
    }

    /// Construct with a custom configuration (test override for the base URL).
    #[must_use]
    pub fn with_config(config: ProviderConfig) -> Self {
        Self {
            config,
            credentials: Arc::new(RwLock::new(None)),
            http: shared_client().clone(),
            subject: None,
        }
    }

    /// A provider that reads `athlete_id` through the credential it is
    /// given, which is a coach's API key: Intervals.icu serves a coach every
    /// athlete who shares with them at that athlete's own path.
    ///
    /// Every athlete-scoped call addresses `athlete_id`, a detail read of an
    /// activity that is not that athlete's is refused as not found, and the
    /// calendar writes are refused: the athlete consented to having their
    /// training read, not written.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for an id Intervals.icu could not have
    /// issued.
    fn delegated(config: ProviderConfig, athlete_id: &str) -> AppResult<Self> {
        check_athlete_id(athlete_id)?;
        info!("Intervals.icu provider initialized for a coached athlete");
        Ok(Self {
            subject: Some(athlete_id.to_owned()),
            ..Self::with_config(config)
        })
    }

    /// The error a call naming `target` that Intervals.icu answered with a
    /// non-success `status` is.
    ///
    /// On a delegated provider the credential is the coach's, so a 401 is
    /// [`coach_credential_expired`] rather than the reader's reconnect, and a
    /// 403 on the athlete's path means the athlete no longer shares with the
    /// coach ([`athlete_off_roster`]). A 403 or 404 on an activity id names
    /// nothing this reader may see. Otherwise [`status_error`].
    fn refusal(&self, op: &str, status: StatusCode, target: Target) -> AppError {
        if self.subject.is_some() {
            match (status, target) {
                (StatusCode::UNAUTHORIZED, _) => {
                    return coach_credential_expired(INTERVALS_ICU, brand());
                }
                (StatusCode::FORBIDDEN, Target::Athlete) => return athlete_off_roster(brand()),
                (StatusCode::FORBIDDEN | StatusCode::NOT_FOUND, Target::Activity) => {
                    return AppError::not_found("Activity");
                }
                _ => {}
            }
        }
        status_error(op, status)
    }

    /// Refuse a calendar write on a delegated provider before anything is
    /// sent.
    fn refuse_delegated_write(&self) -> AppResult<()> {
        if self.subject.is_some() {
            return Err(AppError::invalid_input(
                "A coached athlete's Intervals.icu calendar is read through the coach's \
                 connection, never written",
            ));
        }
        Ok(())
    }

    /// Admit one request against the budget the stored credentials carry,
    /// before it is sent: the signing app's windows and the athlete's grant,
    /// or a personal API key's own.
    async fn admit_request(&self) -> AppResult<()> {
        let budget = request_budget::carried_by(&self.credentials).await;
        request_budget::admit(budget.as_ref(), "intervals_icu").await
    }

    async fn require_credentials(&self) -> AppResult<CallAuth> {
        let guard = self.credentials.read().await;
        let creds = guard.as_ref().ok_or_else(|| {
            AppError::auth_invalid("intervals.icu credentials not set — link your account first")
        })?;
        let secret = creds
            .access_token
            .clone()
            .ok_or_else(|| AppError::auth_invalid("intervals.icu access token missing"))?;
        match creds.kind {
            CredentialKind::OAuthBearer => Ok(CallAuth::Bearer(secret)),
            CredentialKind::ApiKey if creds.client_id.is_empty() => Err(AppError::auth_invalid(
                "intervals.icu athlete id missing (set OAuth2Credentials.client_id to the i123456 athlete id)",
            )),
            CredentialKind::ApiKey => Ok(CallAuth::ApiKey {
                athlete_id: creds.client_id.clone(),
                api_key: secret,
            }),
        }
    }

    /// An athlete-scoped URL: the delegated athlete's path, else the path of
    /// the athlete the credential belongs to.
    fn athlete_url(&self, auth: &CallAuth, suffix: &str) -> String {
        format!(
            "{}/api/v1/athlete/{}{}",
            self.config.api_base_url,
            self.subject
                .as_deref()
                .unwrap_or_else(|| auth.athlete_segment()),
            suffix
        )
    }

    /// The coach roster the stored API key reads (`GET /api/v1/athletes`).
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for an OAuth credential, which
    /// Intervals.icu refuses on this endpoint; [`status_error`] for a refused
    /// call; or the transport or decode failure.
    async fn fetch_coach_roster(&self) -> AppResult<CoachRoster> {
        let auth = self.require_credentials().await?;
        let CallAuth::ApiKey { athlete_id, .. } = &auth else {
            return Err(AppError::invalid_input(
                "Intervals.icu lists a coach's athletes for an API key only",
            ));
        };
        let url = format!("{}/api/v1/athletes", self.config.api_base_url);
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json");
        self.admit_request().await?;
        let response = send_traced(req, "list_athletes", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("list_athletes: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(status_error("list_athletes", response.status()));
        }
        let raw: Vec<IntervalsIcuRosterEntry> = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("list_athletes decode: {e}"))
        })?;
        Ok(coach_roster(athlete_id, raw))
    }

    fn activity_url(&self, activity_id: &str, suffix: &str) -> String {
        format!(
            "{}/api/v1/activity/{}{}",
            self.config.api_base_url, activity_id, suffix
        )
    }

    /// List activities between `oldest` and `newest` (inclusive) — wraps the
    /// `/api/v1/athlete/{id}/activities` endpoint with athlete-scoped Basic
    /// auth.
    ///
    /// Both bounds are required and are serialised with
    /// [`QUERY_DATETIME_FORMAT`]. Callers that hold an open-ended range resolve
    /// it through [`activity_window`] first, so no request can reach the API
    /// with an offset-bearing timestamp or an unbounded range — the two shapes
    /// Intervals.icu answers with 422.
    ///
    /// `oldest` goes on the wire [`QUERY_LOCAL_OFFSET_SLACK_DAYS`] earlier than
    /// asked: the format is read as athlete-local while the argument is a UTC
    /// instant, and the compensation belongs here, at the boundary that owns
    /// the format, rather than in the window the caller reasons about.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing, the upstream
    /// HTTP call fails, or the response cannot be deserialised.
    pub async fn list_activities(
        &self,
        oldest: DateTime<Utc>,
        newest: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<Activity>> {
        let auth = self.require_credentials().await?;
        let query: Vec<(String, String)> = vec![
            ("limit".to_owned(), limit.min(MAX_PAGE_LIMIT).to_string()),
            (
                "oldest".to_owned(),
                (oldest - Duration::days(QUERY_LOCAL_OFFSET_SLACK_DAYS))
                    .format(QUERY_DATETIME_FORMAT)
                    .to_string(),
            ),
            (
                "newest".to_owned(),
                newest.format(QUERY_DATETIME_FORMAT).to_string(),
            ),
        ];
        let url = self.athlete_url(&auth, "/activities");
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json")
            .query(&query);
        self.admit_request().await?;
        let response = send_traced(req, "list_activities", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("list_activities: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(self.refusal("list_activities", response.status(), Target::Athlete));
        }
        let raw: Vec<IntervalsIcuActivity> = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("list_activities decode: {e}"))
        })?;
        Ok(raw
            .into_iter()
            .filter_map(|a| map_activity(a, None, None))
            .collect())
    }

    /// Walk the activity feed backwards through the window until `limit`
    /// activities are collected or the window is exhausted.
    ///
    /// Intervals.icu answers at most [`MAX_PAGE_LIMIT`] activities per request,
    /// and [`Self::list_activities`] clamped the caller's limit to it silently.
    /// That truncation is not local: the historical backfill asks every provider
    /// for two thousand activities and then reads `fetched_count < requested_limit` as
    /// proof the window was exhausted, so a provider that quietly returns 200
    /// makes the backfill record a depth it never reached — and the gate then
    /// serves that shallow slice as a complete season, permanently. Strava and
    /// Garmin already page internally for this reason; this puts Intervals.icu
    /// on the same contract.
    ///
    /// The walk steps `newest` down to the oldest activity of each page
    /// *inclusively* and de-duplicates by activity id. An exclusive step would
    /// be tidier but drops rows: several activities can share one start time,
    /// and a page boundary can fall between them. Repeating a boundary row costs
    /// one duplicate that the id filter removes; skipping past it loses an
    /// activity outright.
    ///
    /// Terminates on any of: the limit reached, a short page (nothing older in
    /// the window), a page contributing no new id (the window stopped advancing),
    /// an exhausted window, or the shared ceiling from [`pages_for`].
    async fn list_activities_paged(
        &self,
        oldest: DateTime<Utc>,
        newest: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<Activity>> {
        let mut collected: Vec<Activity> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut cursor = newest;
        // One page of slack over the caller's limit: the step is inclusive, so
        // each boundary re-delivers a row the id filter then drops, and without
        // it a walk can finish a page short of what was asked for. `pages_for`
        // applies the ceiling every provider shares.
        let pages = pages_for(limit.saturating_add(MAX_PAGE_LIMIT), MAX_PAGE_LIMIT);

        for _ in 0..pages {
            if collected.len() >= limit || cursor <= oldest {
                break;
            }
            let page = self.list_activities(oldest, cursor, MAX_PAGE_LIMIT).await?;
            let page_len = page.len();
            let mut oldest_in_page: Option<DateTime<Utc>> = None;
            let mut added = 0_usize;
            for activity in page {
                let start = activity.start_date();
                oldest_in_page = Some(oldest_in_page.map_or(start, |cur| cur.min(start)));
                if seen.insert(activity.id().to_owned()) {
                    collected.push(activity);
                    added += 1;
                }
            }
            // A short page means the window holds nothing older. `added == 0`
            // means a full page repeated what we already had, so `cursor` is no
            // longer advancing — the guard that makes the pathological
            // same-timestamp feed terminate instead of spinning to the cap.
            if page_len < MAX_PAGE_LIMIT || added == 0 {
                break;
            }
            let Some(oldest_in_page) = oldest_in_page else {
                break;
            };
            cursor = oldest_in_page;
        }

        collected.truncate(limit);
        Ok(collected)
    }

    /// Fetch the per-second time-series streams for a given activity.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing or the upstream
    /// HTTP call fails. HTTP 404 (the activity has no stream data) is a
    /// stream set of zero samples ([`no_recorded_samples`]): the provider's
    /// word that nothing was recorded, never a read that failed.
    pub async fn get_streams(&self, activity_id: &str) -> AppResult<Option<TimeSeriesData>> {
        let auth = self.require_credentials().await?;
        let url = self.activity_url(activity_id, "/streams.json");
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json");
        self.admit_request().await?;
        let response = send_traced(req, "get_streams", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_streams: {e}"))
        })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Some(no_recorded_samples()));
        }
        if !response.status().is_success() {
            return Err(self.refusal("get_streams", response.status(), Target::Activity));
        }
        let raw: Vec<IntervalsIcuStream> = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_streams decode: {e}"))
        })?;
        Ok(Some(streams_to_time_series(&raw)))
    }

    /// Fetch the comments on an activity — its message thread
    /// (`/api/v1/activity/{id}/messages`), oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing, the upstream HTTP
    /// call fails, or the response cannot be deserialised.
    pub async fn get_activity_comments(
        &self,
        activity_id: &str,
    ) -> AppResult<Vec<ActivityComment>> {
        let auth = self.require_credentials().await?;
        let url = self.activity_url(activity_id, "/messages");
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json")
            .query(&[("limit", MAX_ACTIVITY_MESSAGES.to_string())]);
        self.admit_request().await?;
        let response = send_traced(req, "get_activity_comments", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("get_activity_comments: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(self.refusal("get_activity_comments", response.status(), Target::Activity));
        }
        let raw: Vec<IntervalsIcuMessage> = response.json().await.map_err(|e| {
            AppError::external_service(
                "intervals_icu",
                format!("get_activity_comments decode: {e}"),
            )
        })?;
        Ok(comments_from_messages(raw))
    }

    /// The comments on an activity, or `None` when the thread could not be
    /// read. Best-effort: a detail read that lost its comments still serves
    /// the activity, exactly as a lost streams fetch does.
    async fn comments_best_effort(&self, activity_id: &str) -> Option<Vec<ActivityComment>> {
        match self.get_activity_comments(activity_id).await {
            Ok(comments) => Some(comments),
            Err(e) => {
                warn!(activity_id = %activity_id, error = %e, "intervals_icu comments fetch failed; serving the activity without them");
                None
            }
        }
    }

    /// Fetch one activity's raw payload, refused as not found on a delegated
    /// provider when it is not the delegated athlete's.
    async fn fetch_activity(&self, id: &str) -> AppResult<IntervalsIcuActivity> {
        let auth = self.require_credentials().await?;
        let url = self.activity_url(id, "");
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json");
        self.admit_request().await?;
        let response = send_traced(req, "get_activity", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_activity: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(self.refusal("get_activity", response.status(), Target::Activity));
        }
        let raw: IntervalsIcuActivity = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_activity decode: {e}"))
        })?;
        // A coach's key reads every athlete who shares with the coach, so a
        // delegated reader holding one athlete's link could otherwise read
        // another's activity by id. The refusal is a plain not-found: the id
        // names nothing this reader may see.
        match &self.subject {
            Some(athlete) if raw.icu_athlete_id.as_deref() != Some(athlete.as_str()) => {
                Err(AppError::not_found("Activity"))
            }
            _ => Ok(raw),
        }
    }

    /// Fetch the calendar events (planned workouts + races) for the date range.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing or the upstream
    /// HTTP call fails.
    async fn get_events(
        &self,
        oldest: NaiveDate,
        newest: NaiveDate,
    ) -> AppResult<Vec<IntervalsIcuEvent>> {
        let auth = self.require_credentials().await?;
        let url = self.athlete_url(&auth, "/events");
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json")
            .query(&[
                ("oldest", oldest.format("%Y-%m-%d").to_string()),
                ("newest", newest.format("%Y-%m-%d").to_string()),
            ]);
        self.admit_request().await?;
        let response = send_traced(req, "get_events", &url)
            .await
            .map_err(|e| AppError::external_service("intervals_icu", format!("get_events: {e}")))?;
        if !response.status().is_success() {
            return Err(self.refusal("get_events", response.status(), Target::Athlete));
        }
        response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_events decode: {e}"))
        })
    }
}

impl Default for IntervalsIcuProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FitnessProvider for IntervalsIcuProvider {
    fn name(&self) -> &'static str {
        "intervals_icu"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        if credentials.kind == CredentialKind::ApiKey && credentials.client_id.is_empty() {
            return Err(AppError::invalid_input(
                "intervals_icu requires the athlete id (e.g. i123456) in OAuth2Credentials.client_id",
            ));
        }
        let api_key_set = credentials
            .access_token
            .as_ref()
            .is_some_and(|s| !s.is_empty());
        if !api_key_set {
            return Err(AppError::invalid_input(
                "intervals_icu requires a token or API key in OAuth2Credentials.access_token",
            ));
        }
        let mut guard = self.credentials.write().await;
        *guard = Some(credentials);
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        self.credentials.read().await.is_some()
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // Neither tokens nor API keys expire; nothing to refresh.
        Ok(())
    }

    fn set_token_refresh_callback(&self, _callback: TokenRefreshCallback) {
        // No-op — nothing here refreshes.
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        let auth = self.require_credentials().await?;
        let url = self.athlete_url(&auth, "");
        let req = auth
            .authorize(self.http.get(&url))
            .header("Accept", "application/json");
        self.admit_request().await?;
        let response = send_traced(req, "get_athlete", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_athlete: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(self.refusal("get_athlete", response.status(), Target::Athlete));
        }
        let raw: IntervalsIcuAthlete = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_athlete decode: {e}"))
        })?;
        let display_name = raw.name.unwrap_or_else(|| raw.id.clone());
        Ok(Athlete {
            id: raw.id,
            username: raw.email.unwrap_or_default(),
            firstname: Some(display_name),
            lastname: None,
            profile_picture: raw.profile_medium,
            provider: "intervals_icu".to_owned(),
            preferred_units: None,
        })
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        // Not clamped to MAX_PAGE_LIMIT: a caller asking for a season gets a
        // season. The walk bounds itself by MAX_ACTIVITY_PAGES instead.
        let limit = params.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        let (oldest, newest) = activity_window(
            params
                .after
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0)),
            params
                .before
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0)),
        );
        self.list_activities_paged(oldest, newest, limit).await
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let limit = params.limit;
        let (oldest, newest) = activity_window(None, None);
        let activities = self.list_activities(oldest, newest, limit).await?;
        let count = activities.len();
        Ok(CursorPage {
            items: activities,
            next_cursor: None,
            prev_cursor: None,
            has_more: false,
            count,
        })
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        let raw = self.fetch_activity(id).await?;
        map_activity(raw, None, None)
            .ok_or_else(|| AppError::external_service("intervals_icu", "could not map activity"))
    }

    // The comment thread is a second round trip, so the plain read skips it
    // and the detail tier pays it: a detail read is the one that answers
    // "how did that session go", and the comments are the athlete's answer.
    async fn get_activity_detailed(&self, id: &str) -> AppResult<Activity> {
        let raw = self.fetch_activity(id).await?;
        let comments = self.comments_best_effort(id).await;
        map_activity(raw, None, comments)
            .ok_or_else(|| AppError::external_service("intervals_icu", "could not map activity"))
    }

    fn serves_activity_streams(&self) -> bool {
        true
    }

    async fn read_coach_roster(&self) -> AppResult<CoachRoster> {
        self.fetch_coach_roster().await
    }

    // The streams endpoint is a further round trip, so only this tier pays
    // it. Best-effort: a streams failure degrades to the plain activity —
    // stale-less summary beats a dead export.
    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let raw = self.fetch_activity(id).await?;
        let comments = self.comments_best_effort(id).await;
        let streams = match self.get_streams(id).await {
            Ok(streams) => streams,
            Err(e) => {
                warn!(activity_id = %id, error = %e, "intervals_icu streams fetch failed; serving the activity without them");
                None
            }
        };
        map_activity(raw, streams, comments)
            .ok_or_else(|| AppError::external_service("intervals_icu", "could not map activity"))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        // Intervals.icu doesn't expose an aggregate-stats endpoint; derive
        // a 90-day rollup from the activity list so the trait surface
        // returns real data instead of synthetic zeros.
        let (oldest, newest) = activity_window(None, None);
        let activities = self.list_activities(oldest, newest, MAX_PAGE_LIMIT).await?;
        let total_activities = activities.len() as u64;
        let total_distance: f64 = activities
            .iter()
            .map(|a| a.distance_meters().unwrap_or(0.0))
            .sum();
        let total_duration: u64 = activities.iter().map(Activity::duration_seconds).sum();
        let total_elevation_gain: f64 = activities
            .iter()
            .map(|a| a.elevation_gain().unwrap_or(0.0))
            .sum();
        Ok(Stats {
            total_activities,
            total_distance,
            total_duration,
            total_elevation_gain,
            year_to_date: None,
        })
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        self.get_events(from, to)
            .await?
            .into_iter()
            .map(IntervalsIcuEvent::calendar_event_ref)
            .collect()
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        self.refuse_delegated_write()?;
        let auth = self.require_credentials().await?;
        let url = self.athlete_url(&auth, "/events");
        let req = auth
            .authorize(self.http.post(&url))
            .header("Accept", "application/json")
            .json(&event_body(session));
        self.admit_request().await?;
        let response = send_traced(req, "push_planned_session", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("push_planned_session: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(status_error("push_planned_session", response.status()));
        }
        let created: CreatedEvent = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("push_planned_session decode: {e}"))
        })?;
        Ok(created.id.to_string())
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        self.refuse_delegated_write()?;
        let event_id = event_id_segment(provider_event_id)?;
        let auth = self.require_credentials().await?;
        let url = self.athlete_url(&auth, &format!("/events/{event_id}"));
        let req = auth
            .authorize(self.http.put(&url))
            .header("Accept", "application/json")
            .json(&event_body(session));
        self.admit_request().await?;
        let response = send_traced(req, "update_planned_session", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("update_planned_session: {e}"))
            })?;
        response
            .status()
            .is_success()
            .ok_or_else(|| status_error("update_planned_session", response.status()))
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        self.refuse_delegated_write()?;
        if provider_event_ids.is_empty() {
            return Ok(0);
        }
        // By id, never by `external_id`: id deletion is authentication-agnostic,
        // whereas the `external_id` form only reaches events "created by the
        // calling OAuth application", which an API-key link is not. Never the
        // date-range delete either — it would take events Dravr did not write.
        let doomed = provider_event_ids
            .iter()
            .map(|id| event_id_segment(id).map(|n| json!({ "id": n })))
            .collect::<AppResult<Vec<_>>>()?;
        let auth = self.require_credentials().await?;
        let url = self.athlete_url(&auth, "/events/bulk-delete");
        let req = auth
            .authorize(self.http.put(&url))
            .header("Accept", "application/json")
            .json(&doomed);
        self.admit_request().await?;
        let response = send_traced(req, "delete_planned_sessions", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("delete_planned_sessions: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(status_error("delete_planned_sessions", response.status()));
        }
        let deleted: DeleteEventsResponse = response.json().await.map_err(|e| {
            AppError::external_service(
                "intervals_icu",
                format!("delete_planned_sessions decode: {e}"),
            )
        })?;
        Ok(deleted.events_deleted)
    }
}

/// Factory that builds [`IntervalsIcuProvider`] instances for the
/// [`ProviderRegistry`](crate::registry::ProviderRegistry).
pub struct IntervalsIcuProviderFactory;

impl ProviderFactory for IntervalsIcuProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["intervals_icu"]
    }

    // A coach's API key reads every athlete who shares with the coach.
    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        Some(self)
    }
}

impl DelegatedReads for IntervalsIcuProviderFactory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        check_athlete_id(athlete_id)
    }

    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::delegated(
            config, athlete_id,
        )?))
    }
}
