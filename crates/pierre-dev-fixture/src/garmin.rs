// ABOUTME: A local stand-in for Garmin's OAuth2 PKCE pages, its user-id read and the pushes it sends Dravr
// ABOUTME: Renders the synthetic push templates for one Garmin user and posts them to the local webhook route
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin stand-in.
//!
//! A Garmin API connection is made in a browser and fed by pushes, so a
//! local stack needs four things from Garmin, all served here:
//!
//! - `GET /oauth2Confirm`: the consent page. It grants at once and sends the
//!   browser back to the `redirect_uri` with a code and the caller's `state`.
//!   Each authorization stands for a new Garmin user, named in the code.
//! - `POST /di-oauth2-service/oauth/token`: the code and refresh exchanges.
//!   The access token is `devfixture:<garmin user>`, the bearer shape the
//!   rest of the fixture already reads.
//! - `GET /user/id` and `DELETE /user/registration`: who the bearer is, and
//!   the deregistration a disconnect sends.
//! - The pushes. [`GarminFixture::emit`] renders the push templates for one
//!   Garmin user and posts each to Dravr's `/webhooks/garmin/{token}/{type}`
//!   with the `garmin-client-id` header Garmin sends. The binary also emits
//!   the data pushes a few seconds after a new user's first `user/id` read
//!   (the connect's last step), and on `POST /garmin/push/{garmin_user}`.
//!
//! The templates live in `crates/pierre-server/tests/fixtures/garmin/`. They
//! are synthetic: made-up ids and readings in the documented field shapes.
//! Their placeholders are rendered by [`render`]:
//!
//! | Placeholder | Becomes |
//! |---|---|
//! | `"$USER"` | the Garmin user id |
//! | `"$T<±seconds>"` | unix seconds, offset from today's UTC midnight (a number) |
//! | `"$DAY<±days>"` | the `yyyy-mm-dd` date that many days from today (UTC) |
//! | `"$FILES<rest>"` | the activity-file origin followed by `<rest>` |
//!
//! Times are relative so a template never ages out of the windows the reads
//! use. The push URL carries the webhook's path secret, so it is never
//! logged: a push logs its summary type, record count and status only.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::header::{AUTHORIZATION, LOCATION};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Form, Json, Router};
use chrono::{Days, NaiveDate, NaiveTime, Utc};
use serde_json::{json, Map, Value};
use tokio::time::sleep;
use tracing::{info, warn};
use url::Url;

/// Bearer-token prefix the fixture mints and reads; the suffix is the user.
pub const BEARER_PREFIX: &str = "devfixture:";

/// Prefix of the authorization code; the suffix is the Garmin user.
const CODE_PREFIX: &str = "devfixture-code:";

/// Prefix of the refresh token; the suffix is the Garmin user.
const REFRESH_PREFIX: &str = "devfixture-refresh:";

/// Prefix of every Garmin user id an authorization mints.
pub const GARMIN_USER_PREFIX: &str = "devfixture-garmin-";

/// Lifetime of a minted access token, seconds: a day, as Garmin's.
const ACCESS_TOKEN_SECONDS: u64 = 86_400;

/// The header Garmin names Dravr's client in on every push.
const CLIENT_ID_HEADER: &str = "garmin-client-id";

/// Longest a push may take before the fixture gives up on it.
const PUSH_TIMEOUT: Duration = Duration::from_secs(30);

/// One push template: its name, the summary type it posts as, and its body.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PushTemplate {
    /// The template's name: its file stem.
    pub name: &'static str,
    /// The summary type of the push URL and of the body's single key.
    pub summary_type: &'static str,
    /// The body, placeholders unrendered.
    pub body: &'static str,
}

/// A template named `$name` posted as `$summary_type`.
macro_rules! template {
    ($name:literal, $summary_type:literal) => {
        PushTemplate {
            name: $name,
            summary_type: $summary_type,
            body: include_str!(concat!(
                "../../pierre-server/tests/fixtures/garmin/",
                $name,
                ".json"
            )),
        }
    };
}

/// Every push template, one or more per summary type the webhook ingests.
pub(crate) const TEMPLATES: &[PushTemplate] = &[
    template!("activities", "activities"),
    template!("activityDetails", "activityDetails"),
    template!("activityFiles", "activityFiles"),
    template!("dailies", "dailies"),
    template!("sleeps", "sleeps"),
    template!("sleeps.final", "sleeps"),
    template!("hrv", "hrv"),
    template!("userMetrics", "userMetrics"),
    template!("bodyComps", "bodyComps"),
    template!("stressDetails", "stressDetails"),
    template!("skinTemp", "skinTemp"),
    template!("userPermissionsChange", "userPermissionsChange"),
    template!("deregistrations", "deregistrations"),
];

/// The pushes a newly connected dev user receives: every data summary, the
/// sleep already final. Not the activity file (its download needs Garmin's
/// file origin) and not the consent notices, which purge.
pub const DATA_PUSHES: &[&str] = &[
    "activities",
    "activityDetails",
    "dailies",
    "sleeps.final",
    "hrv",
    "userMetrics",
    "bodyComps",
    "stressDetails",
    "skinTemp",
];

/// The template named `name`.
#[must_use]
pub(crate) fn template(name: &str) -> Option<&'static PushTemplate> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Why the fixture could not render or send a push.
#[derive(Debug)]
pub enum FixtureError {
    /// No template carries this name.
    UnknownTemplate(String),
    /// A template is not JSON.
    Template(serde_json::Error),
    /// A `$` string in a template names no placeholder.
    UnknownPlaceholder(String),
    /// The fixture was built without a push target.
    NoPushTarget,
    /// The HTTP client could not be built or the push not sent.
    Http(reqwest::Error),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTemplate(name) => write!(f, "no Garmin push template named {name}"),
            Self::Template(e) => write!(f, "a Garmin push template is not JSON: {e}"),
            Self::UnknownPlaceholder(p) => write!(f, "unknown placeholder {p} in a template"),
            Self::NoPushTarget => f.write_str("the fixture has no push target configured"),
            Self::Http(e) => write!(f, "the push could not be sent: {e}"),
        }
    }
}

impl Error for FixtureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Template(e) => Some(e),
            Self::Http(e) => Some(e),
            Self::UnknownTemplate(_) | Self::UnknownPlaceholder(_) | Self::NoPushTarget => None,
        }
    }
}

/// What a template is rendered against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RenderContext<'ctx> {
    /// The Garmin user every record names.
    pub garmin_user: &'ctx str,
    /// The day `$T` and `$DAY` count from (its UTC midnight).
    pub today: NaiveDate,
    /// The origin activity-file callbacks point at.
    pub files_base: &'ctx str,
}

/// `template` with every placeholder replaced (see the module docs).
///
/// # Errors
///
/// [`FixtureError::Template`] when the template is not JSON, and
/// [`FixtureError::UnknownPlaceholder`] for a `$` string it cannot render.
pub(crate) fn render(template: &str, ctx: &RenderContext<'_>) -> Result<Value, FixtureError> {
    let value: Value = serde_json::from_str(template).map_err(FixtureError::Template)?;
    render_value(value, ctx)
}

fn render_value(value: Value, ctx: &RenderContext<'_>) -> Result<Value, FixtureError> {
    match value {
        Value::String(text) if text.starts_with('$') => render_placeholder(&text, ctx),
        Value::Array(items) => items
            .into_iter()
            .map(|item| render_value(item, ctx))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(fields) => fields
            .into_iter()
            .map(|(key, item)| render_value(item, ctx).map(|item| (key, item)))
            .collect::<Result<Map<_, _>, _>>()
            .map(Value::Object),
        other => Ok(other),
    }
}

fn render_placeholder(text: &str, ctx: &RenderContext<'_>) -> Result<Value, FixtureError> {
    let unknown = || FixtureError::UnknownPlaceholder(text.to_owned());
    if text == "$USER" {
        return Ok(Value::String(ctx.garmin_user.to_owned()));
    }
    if let Some(rest) = text.strip_prefix("$FILES") {
        return Ok(Value::String(format!("{}{rest}", ctx.files_base)));
    }
    if let Some(offset) = text.strip_prefix("$DAY") {
        let days: i64 = offset.parse().map_err(|_| unknown())?;
        let magnitude = Days::new(days.unsigned_abs());
        let date = if days < 0 {
            ctx.today.checked_sub_days(magnitude)
        } else {
            ctx.today.checked_add_days(magnitude)
        };
        return date
            .map(|d| Value::String(d.format("%Y-%m-%d").to_string()))
            .ok_or_else(unknown);
    }
    if let Some(offset) = text.strip_prefix("$T") {
        let seconds: i64 = offset.parse().map_err(|_| unknown())?;
        let midnight = ctx.today.and_time(NaiveTime::MIN).and_utc().timestamp();
        return Ok(json!(midnight + seconds));
    }
    Err(unknown())
}

/// Where the fixture posts its pushes.
#[derive(Clone)]
pub struct PushTarget {
    /// The webhook's base, `…/webhooks/garmin/{token}`: a secret, never logged.
    pub url: String,
    /// Dravr's Garmin client id, sent in the `garmin-client-id` header.
    pub client_id: String,
}

impl fmt::Debug for PushTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushTarget")
            .field("url", &"<redacted>")
            .field("client_id", &self.client_id)
            .finish()
    }
}

/// How the fixture is set up.
#[derive(Debug, Clone, Default)]
pub struct GarminFixtureConfig {
    /// Where pushes go; `None` emits nothing.
    pub push: Option<PushTarget>,
    /// The origin activity-file callbacks point at.
    pub files_base: String,
    /// When set, a new Garmin user's [`DATA_PUSHES`] are emitted this long
    /// after its first `user/id` read: the connect stores the user id with
    /// the token right after that read, and a push naming a user Dravr does
    /// not know yet is dropped.
    pub auto_push_after: Option<Duration>,
}

/// The Garmin stand-in: its configuration and what it has done.
#[derive(Debug, Clone)]
pub struct GarminFixture {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    config: GarminFixtureConfig,
    http: reqwest::Client,
    next_user: AtomicU64,
    auto_pushed: Mutex<HashSet<String>>,
}

impl GarminFixture {
    /// A fixture set up by `config`.
    ///
    /// # Errors
    ///
    /// [`FixtureError::Http`] when the HTTP client cannot be built.
    pub fn try_new(config: GarminFixtureConfig) -> Result<Self, FixtureError> {
        let http = reqwest::Client::builder()
            .timeout(PUSH_TIMEOUT)
            .build()
            .map_err(FixtureError::Http)?;
        // Seeded from the clock so a restarted fixture never mints a Garmin
        // user id an earlier run handed out.
        let seed = u64::try_from(Utc::now().timestamp_millis()).unwrap_or_default();
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                http,
                next_user: AtomicU64::new(seed),
                auto_pushed: Mutex::new(HashSet::new()),
            }),
        })
    }

    /// A new Garmin user id, never handed out before by this fixture.
    fn mint_user(&self) -> String {
        let n = self.inner.next_user.fetch_add(1, Ordering::Relaxed);
        format!("{GARMIN_USER_PREFIX}{n}")
    }

    /// Render the templates `names` for `garmin_user` and post each, in
    /// order, to the push target. Returns each template's name and the
    /// webhook's status code.
    ///
    /// # Errors
    ///
    /// [`FixtureError::NoPushTarget`] without a target, and the first
    /// template or send failure otherwise; pushes before it were sent.
    pub async fn emit(
        &self,
        garmin_user: &str,
        names: &[&str],
    ) -> Result<Vec<(String, u16)>, FixtureError> {
        let target = self
            .inner
            .config
            .push
            .as_ref()
            .ok_or(FixtureError::NoPushTarget)?;
        let ctx = RenderContext {
            garmin_user,
            today: Utc::now().date_naive(),
            files_base: &self.inner.config.files_base,
        };
        let mut sent = Vec::with_capacity(names.len());
        for name in names {
            let template =
                template(name).ok_or_else(|| FixtureError::UnknownTemplate((*name).to_owned()))?;
            let body = render(template.body, &ctx)?;
            let records = body
                .get(template.summary_type)
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let status = self
                .inner
                .http
                .post(format!("{}/{}", target.url, template.summary_type))
                .header(CLIENT_ID_HEADER, &target.client_id)
                .json(&body)
                .send()
                .await
                // The push URL carries the webhook token as a path segment,
                // and a reqwest error prints its URL: drop it before the
                // error can reach a log.
                .map_err(|e| FixtureError::Http(e.without_url()))?
                .status()
                .as_u16();
            info!(
                template = template.name,
                summary_type = template.summary_type,
                records,
                status,
                "sent a Garmin push"
            );
            sent.push(((*name).to_owned(), status));
        }
        Ok(sent)
    }

    /// Emit [`DATA_PUSHES`] for `garmin_user` after the configured delay,
    /// once per user.
    fn schedule_auto_push(&self, garmin_user: &str) {
        let Some(delay) = self.inner.config.auto_push_after else {
            return;
        };
        if self.inner.config.push.is_none() {
            return;
        }
        let first = self
            .inner
            .auto_pushed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(garmin_user.to_owned());
        if !first {
            return;
        }
        let fixture = self.clone();
        let user = garmin_user.to_owned();
        tokio::spawn(async move {
            sleep(delay).await;
            if let Err(e) = fixture.emit(&user, DATA_PUSHES).await {
                warn!(garmin_user = %user, error = %e, "the automatic Garmin pushes failed");
            }
        });
    }
}

/// Every Garmin route the fixture answers.
pub fn router(fixture: GarminFixture) -> Router {
    Router::new()
        .route("/oauth2Confirm", get(authorize))
        .route("/di-oauth2-service/oauth/token", post(token))
        .route("/user/id", get(user_id))
        .route("/user/registration", delete(deregister))
        .route("/garmin/push/{garmin_user}", post(push_now))
        .with_state(fixture)
}

/// The user an `Authorization: Bearer devfixture:<id>` header names.
#[must_use]
pub fn user_from_bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ").unwrap_or(value);
    token.strip_prefix(BEARER_PREFIX).map(ToOwned::to_owned)
}

/// An `OAuth2` error body.
fn oauth_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

/// `GET /oauth2Confirm`: consent granted at once for a new Garmin user. The
/// request must be a PKCE code request with a redirect and a state, as
/// Garmin requires.
async fn authorize(
    State(fixture): State<GarminFixture>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (Some(redirect), Some(state)) = (params.get("redirect_uri"), params.get("state")) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if params.get("response_type").map(String::as_str) != Some("code")
        || !params.contains_key("code_challenge")
    {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let Ok(mut location) = Url::parse(redirect) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let garmin_user = fixture.mint_user();
    location
        .query_pairs_mut()
        .append_pair("code", &format!("{CODE_PREFIX}{garmin_user}"))
        .append_pair("state", state);
    info!(garmin_user = %garmin_user, "granted a Garmin authorization");
    (StatusCode::FOUND, [(LOCATION, location.to_string())]).into_response()
}

/// `POST /di-oauth2-service/oauth/token`: the code and refresh exchanges.
async fn token(Form(form): Form<HashMap<String, String>>) -> Response {
    let user = match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => form
            .get("code")
            .and_then(|code| code.strip_prefix(CODE_PREFIX))
            .filter(|_| form.contains_key("code_verifier")),
        Some("refresh_token") => form
            .get("refresh_token")
            .and_then(|refresh| refresh.strip_prefix(REFRESH_PREFIX)),
        _ => None,
    };
    let Some(user) = user else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    Json(json!({
        "access_token": format!("{BEARER_PREFIX}{user}"),
        "token_type": "bearer",
        "refresh_token": format!("{REFRESH_PREFIX}{user}"),
        "expires_in": ACCESS_TOKEN_SECONDS,
        "scope": "PARTNER_READ CONNECT_READ",
        "refresh_token_expires_in": ACCESS_TOKEN_SECONDS * 90,
    }))
    .into_response()
}

/// `GET /user/id`: the Garmin user behind the bearer. A user's first read
/// is the connect's last step, so it schedules the automatic pushes.
async fn user_id(State(fixture): State<GarminFixture>, headers: HeaderMap) -> Response {
    let Some(user) = user_from_bearer(&headers) else {
        warn!("missing or malformed bearer; refusing the Garmin user id");
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.starts_with(GARMIN_USER_PREFIX) {
        fixture.schedule_auto_push(&user);
    }
    Json(json!({ "userId": user })).into_response()
}

/// `DELETE /user/registration`: the deregistration a disconnect sends.
async fn deregister(headers: HeaderMap) -> StatusCode {
    user_from_bearer(&headers).map_or(StatusCode::UNAUTHORIZED, |user| {
        info!(garmin_user = %user, "deregistered a Garmin user");
        StatusCode::NO_CONTENT
    })
}

/// `POST /garmin/push/{garmin_user}?templates=a,b`: send those pushes now,
/// or [`DATA_PUSHES`] when none are named.
async fn push_now(
    State(fixture): State<GarminFixture>,
    Path(garmin_user): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let requested: Vec<&str> = params
        .get("templates")
        .map(|list| list.split(',').filter(|n| !n.is_empty()).collect())
        .unwrap_or_default();
    let names = if requested.is_empty() {
        DATA_PUSHES.to_vec()
    } else {
        requested
    };
    match fixture.emit(&garmin_user, &names).await {
        Ok(sent) => Json(json!({
            "sent": sent
                .into_iter()
                .map(|(name, status)| json!({ "template": name, "status": status }))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(FixtureError::NoPushTarget) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e @ (FixtureError::UnknownTemplate(_) | FixtureError::UnknownPlaceholder(_))) => {
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RenderContext<'static> {
        RenderContext {
            garmin_user: "devfixture-garmin-1",
            today: NaiveDate::from_ymd_opt(2026, 10, 27).unwrap_or_default(),
            files_base: "http://127.0.0.1:9555/wellness-api/rest",
        }
    }

    #[test]
    fn every_template_renders_as_one_list_of_its_summary_type() {
        for template in TEMPLATES {
            let body = render(template.body, &ctx())
                .unwrap_or_else(|e| panic!("{} renders: {e}", template.name));
            let object = body.as_object().expect("a JSON object");
            assert_eq!(object.len(), 1, "{} has one key", template.name);
            let records = object
                .get(template.summary_type)
                .and_then(Value::as_array)
                .unwrap_or_else(|| {
                    panic!("{} is a list of {}", template.name, template.summary_type)
                });
            assert!(!records.is_empty(), "{} carries records", template.name);
            for record in records {
                assert_eq!(record["userId"], "devfixture-garmin-1", "{}", template.name);
            }
            assert!(
                !body.to_string().contains("\"$"),
                "{} keeps no placeholder",
                template.name
            );
        }
    }

    #[test]
    fn times_and_days_count_from_todays_utc_midnight() {
        let body = render(
            r#"{"x":[{"t":"$T-3600","d":"$DAY-1","n":"$DAY+0","f":"$FILES/f?id=1"}]}"#,
            &ctx(),
        )
        .expect("renders");
        let record = &body["x"][0];
        // 2026-10-27T00:00:00Z is 1_793_059_200.
        assert_eq!(record["t"], 1_793_059_200 - 3_600);
        assert_eq!(record["d"], "2026-10-26");
        assert_eq!(record["n"], "2026-10-27");
        assert_eq!(
            record["f"],
            "http://127.0.0.1:9555/wellness-api/rest/f?id=1"
        );
    }

    #[test]
    fn an_unknown_placeholder_is_refused() {
        assert!(matches!(
            render(r#"{"x":[{"v":"$NOPE"}]}"#, &ctx()),
            Err(FixtureError::UnknownPlaceholder(_))
        ));
        assert!(matches!(
            render(r#"{"x":[{"v":"$Tabc"}]}"#, &ctx()),
            Err(FixtureError::UnknownPlaceholder(_))
        ));
    }

    #[test]
    fn every_data_push_names_a_template() {
        for name in DATA_PUSHES {
            assert!(template(name).is_some(), "{name}");
        }
    }

    #[tokio::test]
    async fn the_code_and_refresh_exchanges_name_the_authorized_user() {
        let mut form = HashMap::new();
        form.insert("grant_type".to_owned(), "authorization_code".to_owned());
        form.insert(
            "code".to_owned(),
            format!("{CODE_PREFIX}devfixture-garmin-7"),
        );
        form.insert("code_verifier".to_owned(), "verifier".to_owned());
        let response = token(Form(form)).await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut refresh = HashMap::new();
        refresh.insert("grant_type".to_owned(), "refresh_token".to_owned());
        refresh.insert("refresh_token".to_owned(), "someone-else".to_owned());
        assert_eq!(token(Form(refresh)).await.status(), StatusCode::BAD_REQUEST);

        let mut no_verifier = HashMap::new();
        no_verifier.insert("grant_type".to_owned(), "authorization_code".to_owned());
        no_verifier.insert(
            "code".to_owned(),
            format!("{CODE_PREFIX}devfixture-garmin-7"),
        );
        assert_eq!(
            token(Form(no_verifier)).await.status(),
            StatusCode::BAD_REQUEST,
            "a code exchanged without the PKCE verifier is refused"
        );
    }
}
