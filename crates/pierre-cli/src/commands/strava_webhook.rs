// ABOUTME: `pierre-cli strava-webhook` — register, list and delete the Strava push subscription
// ABOUTME: Talks to Strava's push_subscriptions API with the app credentials from env; /webhooks/strava is the callback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Registration of the Strava push subscription.
//!
//! Strava delivers activity events only to an app that has registered ONE
//! push subscription (per app, not per athlete) through its
//! `push_subscriptions` API. The platform's `/webhooks/strava` route answers
//! the subscription's verification challenge and the events, but nothing
//! registered the subscription — so no event ever arrived. These verbs are
//! that registration, run by an operator with the app's credentials:
//!
//! - `subscribe` POSTs `client_id`, `client_secret`, `callback_url` and
//!   `verify_token`; Strava immediately GETs the callback with a challenge,
//!   which the running server answers when its `STRAVA_WEBHOOK_VERIFY_TOKEN`
//!   matches the token sent here.
//! - `list` shows the app's subscription, if any.
//! - `delete` removes one by id.
//!
//! Credentials come from `STRAVA_CLIENT_ID`, `STRAVA_CLIENT_SECRET` and
//! `STRAVA_WEBHOOK_VERIFY_TOKEN` — the same variables the server reads — and
//! are never printed. `PIERRE_STRAVA_API_BASE_URL` redirects the API base,
//! the seam the server's Strava provider honours too.

use std::env;
use std::time::Duration;

use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// Strava's API base, the default behind `PIERRE_STRAVA_API_BASE_URL`.
const STRAVA_API_BASE_URL: &str = "https://www.strava.com/api/v3";

/// Path of the subscription callback on the platform. The frontend door
/// (nginx) proxies `/webhooks/` to the backend, so the public base URL plus
/// this path reaches the server's handler.
const CALLBACK_PATH: &str = "/webhooks/strava";

/// Per-request timeout; Strava verifies the callback synchronously inside the
/// subscribe call, which may take a few seconds.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// `pierre-cli strava-webhook` — the app's push subscription.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum StravaWebhookCommand {
    /// Register the push subscription pointing at `<base-url>/webhooks/strava`
    Subscribe {
        /// Public base URL of the deployment the callback reaches (env: `BASE_URL`)
        #[arg(long, env = "BASE_URL")]
        base_url: String,
    },
    /// Show the app's current subscription (Strava allows one per app)
    List,
    /// Delete a subscription by id
    Delete {
        /// The subscription id, as `list` prints it
        #[arg(long)]
        id: u64,
    },
}

/// The Strava app credentials the `push_subscriptions` API authenticates with.
pub struct StravaApp {
    /// The app's public `client_id`.
    pub client_id: String,
    /// The app's `client_secret`; sent to Strava over TLS, never printed.
    pub client_secret: String,
    /// The token Strava echoes in the callback's `hub.verify_token`; required
    /// for `subscribe` only, and `None` when the variable is unset.
    pub verify_token: Option<String>,
}

impl StravaApp {
    /// Read the app credentials from `STRAVA_CLIENT_ID`, `STRAVA_CLIENT_SECRET`
    /// and `STRAVA_WEBHOOK_VERIFY_TOKEN`.
    ///
    /// # Errors
    /// Returns an error naming the missing variable when the id or secret is
    /// unset.
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            client_id: required_env("STRAVA_CLIENT_ID")?,
            client_secret: required_env("STRAVA_CLIENT_SECRET")?,
            verify_token: env::var("STRAVA_WEBHOOK_VERIFY_TOKEN")
                .ok()
                .filter(|token| !token.is_empty()),
        })
    }
}

fn required_env(key: &str) -> AppResult<String> {
    env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::invalid_input(format!("{key} is not set")))
}

/// The HTTP method of a [`StravaRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Create the subscription (form-encoded body).
    Post,
    /// List the app's subscriptions (credentials in the query string).
    Get,
    /// Delete one subscription (credentials in the query string).
    Delete,
}

/// One request to Strava's `push_subscriptions` API, built without sending so
/// its exact shape is checkable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StravaRequest {
    /// HTTP method.
    pub method: Method,
    /// Full URL, query string excluded.
    pub url: String,
    /// Query parameters, in the order they are sent.
    pub query: Vec<(&'static str, String)>,
    /// Form fields (`application/x-www-form-urlencoded`), in the order they
    /// are sent; empty for GET and DELETE.
    pub form: Vec<(&'static str, String)>,
}

/// The callback URL a subscription for `base_url` registers.
///
/// A trailing slash on the base is dropped so the path is never doubled.
#[must_use]
pub fn callback_url(base_url: &str) -> String {
    format!("{}{CALLBACK_PATH}", base_url.trim_end_matches('/'))
}

fn subscriptions_url(api_base: &str) -> String {
    format!("{}/push_subscriptions", api_base.trim_end_matches('/'))
}

/// The `POST /push_subscriptions` that registers the callback.
///
/// # Errors
/// Returns an error when the app carries no verify token: Strava requires
/// one, and the server compares it against `STRAVA_WEBHOOK_VERIFY_TOKEN`.
pub fn subscribe_request(
    api_base: &str,
    app: &StravaApp,
    base_url: &str,
) -> AppResult<StravaRequest> {
    let verify_token = app.verify_token.clone().ok_or_else(|| {
        AppError::invalid_input(
            "STRAVA_WEBHOOK_VERIFY_TOKEN is not set; the server checks the callback against it",
        )
    })?;
    Ok(StravaRequest {
        method: Method::Post,
        url: subscriptions_url(api_base),
        query: Vec::new(),
        form: vec![
            ("client_id", app.client_id.clone()),
            ("client_secret", app.client_secret.clone()),
            ("callback_url", callback_url(base_url)),
            ("verify_token", verify_token),
        ],
    })
}

/// The `GET /push_subscriptions` that lists the app's subscription.
#[must_use]
pub fn list_request(api_base: &str, app: &StravaApp) -> StravaRequest {
    StravaRequest {
        method: Method::Get,
        url: subscriptions_url(api_base),
        query: credentials_query(app),
        form: Vec::new(),
    }
}

/// The `DELETE /push_subscriptions/{id}` that removes one subscription.
#[must_use]
pub fn delete_request(api_base: &str, app: &StravaApp, id: u64) -> StravaRequest {
    StravaRequest {
        method: Method::Delete,
        url: format!("{}/{id}", subscriptions_url(api_base)),
        query: credentials_query(app),
        form: Vec::new(),
    }
}

fn credentials_query(app: &StravaApp) -> Vec<(&'static str, String)> {
    vec![
        ("client_id", app.client_id.clone()),
        ("client_secret", app.client_secret.clone()),
    ]
}

/// Send a built request and return the status with the parsed body (`Null`
/// for an empty body, as DELETE answers 204).
async fn send(request: &StravaRequest) -> AppResult<(reqwest::StatusCode, Value)> {
    let http = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("pierre-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::internal(format!("Failed to build HTTP client: {e}")))?;

    let builder = match request.method {
        Method::Post => http.post(&request.url).form(&request.form),
        Method::Get => http.get(&request.url),
        Method::Delete => http.delete(&request.url),
    };
    let response = builder
        .query(&request.query)
        .send()
        .await
        .map_err(|e| AppError::external_service("strava", format!("request failed: {e}")))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| {
        AppError::external_service("strava", format!("could not read response body: {e}"))
    })?;
    let body = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    Ok((status, body))
}

/// Turn a non-2xx Strava answer into an error carrying its status and body.
///
/// Strava's error bodies name the rejected field (`callback_url`, `verify
/// token`), never the credentials, so the body is safe to surface.
fn rejected(status: reqwest::StatusCode, body: &Value) -> AppError {
    AppError::external_service(
        "strava",
        format!("push_subscriptions answered {status}: {body}"),
    )
}

/// `strava-webhook subscribe` — register the callback for `base_url`.
async fn subscribe(api_base: &str, base_url: &str) -> AppResult<()> {
    let app = StravaApp::from_env()?;
    let request = subscribe_request(api_base, &app, base_url)?;
    let callback = callback_url(base_url);
    println!("  Registering Strava push subscription for {callback}");
    println!("  (Strava verifies the callback now; the server at that URL must be up with the same STRAVA_WEBHOOK_VERIFY_TOKEN)");

    let (status, body) = send(&request).await?;
    if !status.is_success() {
        return Err(rejected(status, &body));
    }
    match body.get("id").and_then(Value::as_u64) {
        Some(id) => println!("  Subscribed: id={id}"),
        None => println!("  Subscribed: {body}"),
    }
    Ok(())
}

/// `strava-webhook list` — show the app's subscription.
async fn list(api_base: &str) -> AppResult<()> {
    let app = StravaApp::from_env()?;
    let (status, body) = send(&list_request(api_base, &app)).await?;
    if !status.is_success() {
        return Err(rejected(status, &body));
    }
    match body.as_array() {
        Some(subscriptions) if !subscriptions.is_empty() => {
            println!("  Strava push subscriptions ({}):", subscriptions.len());
            for subscription in subscriptions {
                let id = subscription.get("id").and_then(Value::as_u64).unwrap_or(0);
                let callback = subscription
                    .get("callback_url")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let created = subscription
                    .get("created_at")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                println!("    - id={id}  {callback}  created={created}");
            }
        }
        _ => println!("  No push subscription registered for this app."),
    }
    Ok(())
}

/// `strava-webhook delete` — remove one subscription.
async fn delete(api_base: &str, id: u64) -> AppResult<()> {
    let app = StravaApp::from_env()?;
    let (status, body) = send(&delete_request(api_base, &app, id)).await?;
    if !status.is_success() {
        return Err(rejected(status, &body));
    }
    println!("  Deleted Strava push subscription id={id}");
    Ok(())
}

/// Dispatch a `strava-webhook` verb.
///
/// # Errors
/// Returns an error when a credential variable is unset, the request cannot
/// be sent, or Strava rejects it.
pub async fn dispatch(command: StravaWebhookCommand) -> AppResult<()> {
    let api_base =
        env::var("PIERRE_STRAVA_API_BASE_URL").unwrap_or_else(|_| STRAVA_API_BASE_URL.to_owned());
    match command {
        StravaWebhookCommand::Subscribe { base_url } => subscribe(&api_base, &base_url).await,
        StravaWebhookCommand::List => list(&api_base).await,
        StravaWebhookCommand::Delete { id } => delete(&api_base, id).await,
    }
}
