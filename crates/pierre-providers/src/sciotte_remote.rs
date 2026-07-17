// ABOUTME: HTTP client for the dedicated dravr-sciotte scraper service (ADR-021).
// ABOUTME: Reads DRAVR_SCIOTTE_REMOTE_URL + DRAVR_SCIOTTE_API_KEY; the sole scrape path since the Phase 4 cutover.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote sciotte scraper client.
//!
//! The interactive login **and** the activity/athlete scrape run on a dedicated
//! `dravr-sciotte-server`, isolating the memory-heavy headless Chrome off the
//! multi-tenant API pod (see [[ADR-021]]). Since the Phase 4 cutover this is the
//! only scrape path: `DRAVR_SCIOTTE_REMOTE_URL` must be set or every sciotte call
//! errors — there is no in-process fallback.
//!
//! ## Session-of-record boundary
//!
//! The scraper service holds sessions only transiently (in-memory + ephemeral
//! disk). The **platform stays the session-of-record**: on a successful login it
//! [`export_session`](RemoteSciotteClient::export_session)s the full
//! `AuthSession` and persists it (KMS-encrypted `oauth_tokens`), then
//! [`import_session`](RemoteSciotteClient::import_session)s it back before a
//! scrape when the service has scaled to zero / been redeployed. This keeps the
//! platform's sessions durable — no re-login churn on redeploy.
//!
//! ## Auth
//!
//! Requests carry the `DRAVR_SCIOTTE_API_KEY` bearer the server gates on. The
//! key is read once at construction; it is never logged.

use std::env;
use std::time::Duration;

use dravr_sciotte::models::{Activity as SciotteActivity, AuthSession};
use pierre_core::errors::{AppError, AppResult};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

/// Environment variable holding the remote scraper's base URL. Required since
/// the Phase 4 cutover — unset makes `require_from_env` error (no fallback).
pub const ENV_REMOTE_URL: &str = "DRAVR_SCIOTTE_REMOTE_URL";
/// Environment variable holding the bearer the scraper service gates on.
pub const ENV_API_KEY: &str = "DRAVR_SCIOTTE_API_KEY";

/// Outcome of an interactive-login step, mirroring the scraper service's
/// `{status, ...}` login response.
///
/// Continuation variants carry the server-minted `flow_id` naming the parked
/// login flow — echoed back on `submit_otp`/`select_2fa` so a multi-provider,
/// multi-user service resumes the right browser (the server falls back to its
/// sole pending flow when the id is absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteLoginOutcome {
    /// Login complete; the service now holds the session under `session_id`.
    /// The caller exports + persists it, never leaving it only server-side.
    Authenticated {
        /// Server-side session id to export the full session with.
        session_id: String,
        /// Provider the session authenticates against (`"garmin"`, `"strava"`).
        /// The caller maps it to the backend name and persists under it — so
        /// the platform never tracks the provider across the multi-step flow.
        provider: String,
    },
    /// A one-time password / 2FA code is required (`submit_otp`).
    OtpRequired {
        /// Parked flow to resume with the code.
        flow_id: Option<String>,
    },
    /// The user must pick a 2FA method (`select_2fa`); options passed through
    /// verbatim so the caller renders the same choice the in-process path does.
    TwoFactorChoice {
        /// The provider's 2FA method options, verbatim.
        options: Value,
        /// Parked flow to resume with the selection.
        flow_id: Option<String>,
    },
    /// A number-matching challenge — the user approves the number on their phone.
    NumberMatch {
        /// The number the user must confirm on their device.
        number: String,
        /// Parked flow the challenge belongs to.
        flow_id: Option<String>,
    },
    /// The provider rejected the credentials / flow.
    Failed(String),
}

/// Athlete profile fields the platform needs, as reported by `GET /api/athlete`.
///
/// A subset of the scraper's profile DTO (serde ignores the rest) so the client
/// does not have to mirror the full upstream type.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteAthleteProfile {
    /// Display name, when the provider exposes one.
    #[serde(default)]
    pub display_name: Option<String>,
    /// First name, when available.
    #[serde(default)]
    pub firstname: Option<String>,
    /// Last name, when available.
    #[serde(default)]
    pub lastname: Option<String>,
    /// Profile picture URL, when available.
    #[serde(default)]
    pub profile_picture_url: Option<String>,
}

/// Query knobs for a remote activity fetch. Mirrors the `ActivityParams`
/// subset the scraper service honours.
#[derive(Debug, Clone, Default)]
pub struct RemoteActivityQuery {
    /// Max activities to return.
    pub limit: Option<u32>,
    /// Lower bound (epoch seconds) — only activities on/after this instant.
    pub after_epoch: Option<i64>,
    /// Upper bound (epoch seconds) — only activities on/before this instant.
    pub before_epoch: Option<i64>,
    /// Optional sport-type filter.
    pub sport_type: Option<String>,
    /// Whether to enrich each activity with its detail page (N+1, slower).
    pub enrich_details: bool,
}

/// Client for the dedicated `dravr-sciotte-server`.
///
/// Cheap to clone (wraps a connection-pooled [`reqwest::Client`]); construct via
/// [`require_from_env`](Self::require_from_env) at the call site — the scrape runs
/// only on the service since the Phase 4 cutover.
#[derive(Clone)]
pub struct RemoteSciotteClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl RemoteSciotteClient {
    /// Build a client from the environment, or `None` when `DRAVR_SCIOTTE_REMOTE_URL`
    /// is unset or empty. Scrape-path callers use
    /// [`require_from_env`](Self::require_from_env), which turns the unset case into
    /// an error — since the Phase 4 cutover the service is the only scrape path, so
    /// a missing URL is a deployment fault, not a fallback signal.
    ///
    /// # Errors
    ///
    /// Returns an error only when the HTTP client itself cannot be built.
    pub fn from_env() -> AppResult<Option<Self>> {
        let Ok(base_url) = env::var(ENV_REMOTE_URL) else {
            return Ok(None);
        };
        let base_url = base_url.trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return Ok(None);
        }
        let http = Client::builder()
            // The 2FA flow parks a live browser across this one request; the HTTP
            // timeout must outlast the scraper's own login window (300s) plus
            // headroom so the platform doesn't cut the call before the scraper does.
            .timeout(Duration::from_secs(330))
            .build()
            .map_err(|e| AppError::internal(format!("build sciotte http client: {e}")))?;
        let api_key = env::var(ENV_API_KEY).ok().filter(|k| !k.is_empty());
        debug!(base_url = %base_url, "Remote sciotte client enabled");
        Ok(Some(Self {
            http,
            base_url,
            api_key,
        }))
    }

    /// Build a client, erroring when `DRAVR_SCIOTTE_REMOTE_URL` is unset.
    ///
    /// Since the Phase 4 cutover the sciotte scrape runs *only* on the
    /// dedicated service ([[ADR-021]]) — there is no in-process fallback — so a
    /// missing URL is a deployment misconfiguration, surfaced as an internal
    /// error rather than silently doing nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is unset/empty or the HTTP client can't be built.
    pub fn require_from_env() -> AppResult<Self> {
        Self::from_env()?.ok_or_else(|| {
            AppError::internal(
                "DRAVR_SCIOTTE_REMOTE_URL must be set — the in-process sciotte \
                 scrape path was removed in the ADR-021 Phase 4 cutover",
            )
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base_url));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        req
    }

    /// POST `/auth/login-with-credentials` — start an interactive login on
    /// `provider` (`"garmin"`, `"strava"`): the multi-provider service needs
    /// it named on every new flow.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or an unparseable response.
    pub async fn login_with_credentials(
        &self,
        email: &str,
        password: &str,
        method: &str,
        provider: &str,
    ) -> AppResult<RemoteLoginOutcome> {
        let body = serde_json::json!({
            "email": email,
            "password": password,
            "method": method,
            "provider": provider,
        });
        let resp = self
            .request(reqwest::Method::POST, "/auth/login-with-credentials")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte login request: {e}")))?;
        Self::parse_login_outcome(resp).await
    }

    /// POST `/auth/submit-otp` — continue an interactive login with an OTP/2FA
    /// code. `flow_id` names the parked flow (from the continuation outcome);
    /// `None` lets the server resume its sole pending flow.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or an unparseable response.
    pub async fn submit_otp(
        &self,
        code: &str,
        flow_id: Option<&str>,
    ) -> AppResult<RemoteLoginOutcome> {
        let resp = self
            .request(reqwest::Method::POST, "/auth/submit-otp")
            .json(&serde_json::json!({ "code": code, "flow_id": flow_id }))
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte submit-otp request: {e}")))?;
        Self::parse_login_outcome(resp).await
    }

    /// POST `/auth/select-2fa` — pick a 2FA method during an interactive login.
    /// `flow_id` names the parked flow; `None` lets the server resume its sole
    /// pending flow.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or an unparseable response.
    pub async fn select_2fa(
        &self,
        option_id: &str,
        flow_id: Option<&str>,
    ) -> AppResult<RemoteLoginOutcome> {
        let resp = self
            .request(reqwest::Method::POST, "/auth/select-2fa")
            .json(&serde_json::json!({ "option_id": option_id, "flow_id": flow_id }))
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte select-2fa request: {e}")))?;
        Self::parse_login_outcome(resp).await
    }

    /// GET `/auth/sessions/{id}/export` — retrieve the full `AuthSession` so the
    /// platform can persist it as the durable session-of-record. The service
    /// returns `{provider, session}`; the provider is dropped here because the
    /// caller already knows it from the `Authenticated` outcome.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status, or a body
    /// that does not deserialize into the export shape.
    pub async fn export_session(&self, session_id: &str) -> AppResult<AuthSession> {
        #[derive(Deserialize)]
        struct ExportResponse {
            session: AuthSession,
        }
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!("/auth/sessions/{session_id}/export"),
            )
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte export request: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::internal(format!(
                "sciotte export returned {}",
                resp.status()
            )));
        }
        Ok(resp
            .json::<ExportResponse>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte export decode: {e}")))?
            .session)
    }

    /// POST `/auth/import-session` — re-hydrate the service's transient store from
    /// the platform's durable session before a scrape, tagged with the provider
    /// (`"garmin"`, `"strava"`) so the multi-provider service routes its scrapes.
    /// Idempotent; returns the session id the scrape endpoints key on via
    /// `X-Session-Id`.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or a non-success status.
    pub async fn import_session(&self, session: &AuthSession, provider: &str) -> AppResult<String> {
        #[derive(Deserialize)]
        struct ImportResponse {
            session_id: String,
        }
        let resp = self
            .request(reqwest::Method::POST, "/auth/import-session")
            .json(&serde_json::json!({ "provider": provider, "session": session }))
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte import request: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::internal(format!(
                "sciotte import returned {}",
                resp.status()
            )));
        }
        Ok(resp
            .json::<ImportResponse>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte import decode: {e}")))?
            .session_id)
    }

    /// GET `/api/activities` — scrape the activity list for `session_id`.
    ///
    /// Returns the raw upstream [`SciotteActivity`] rows; the caller maps them
    /// with `convert_activity`, so there is no duplicate DTO logic.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status (e.g. the
    /// service no longer holds the session — the caller re-imports and retries),
    /// or an unparseable body.
    pub async fn get_activities(
        &self,
        session_id: &str,
        query: &RemoteActivityQuery,
    ) -> AppResult<Vec<SciotteActivity>> {
        #[derive(Deserialize)]
        struct ActivitiesResponse {
            activities: Vec<SciotteActivity>,
        }
        let mut req = self
            .request(reqwest::Method::GET, "/api/activities")
            .header("X-Session-Id", session_id);
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(l) = query.limit {
            params.push(("limit", l.to_string()));
        }
        if let Some(a) = query.after_epoch {
            params.push(("after", a.to_string()));
        }
        if let Some(b) = query.before_epoch {
            params.push(("before", b.to_string()));
        }
        if let Some(s) = &query.sport_type {
            params.push(("sport_type", s.clone()));
        }
        if query.enrich_details {
            params.push(("detail", "true".to_owned()));
        }
        if !params.is_empty() {
            req = req.query(&params);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte activities request: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "sciotte activities returned {status}"
            )));
        }
        Ok(resp
            .json::<ActivitiesResponse>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte activities decode: {e}")))?
            .activities)
    }

    /// GET `/api/athlete` — scrape the athlete profile for `session_id`.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status, or an
    /// unparseable body.
    pub async fn get_athlete(&self, session_id: &str) -> AppResult<RemoteAthleteProfile> {
        let resp = self
            .request(reqwest::Method::GET, "/api/athlete")
            .header("X-Session-Id", session_id)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte athlete request: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "sciotte athlete returned {status}"
            )));
        }
        resp.json::<RemoteAthleteProfile>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte athlete decode: {e}")))
    }

    /// GET `/api/activities/{id}` — scrape a single activity's detail for `session_id`.
    ///
    /// Returns the raw upstream [`SciotteActivity`]; the caller reuses the same
    /// `convert_activity` mapping the list path uses.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status, or an
    /// unparseable body.
    pub async fn get_activity(
        &self,
        session_id: &str,
        activity_id: &str,
    ) -> AppResult<SciotteActivity> {
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!("/api/activities/{activity_id}"),
            )
            .header("X-Session-Id", session_id)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("sciotte activity request: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "sciotte activity returned {status}"
            )));
        }
        resp.json::<SciotteActivity>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte activity decode: {e}")))
    }

    /// Parse a login-step response body into a [`RemoteLoginOutcome`].
    async fn parse_login_outcome(resp: reqwest::Response) -> AppResult<RemoteLoginOutcome> {
        let http_status = resp.status();
        let body = resp.json::<Value>().await.map_err(|e| {
            AppError::internal(format!("sciotte login decode (HTTP {http_status}): {e}"))
        })?;
        let status = body.get("status").and_then(Value::as_str).unwrap_or("");
        let flow_id = body
            .get("flow_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match status {
            "authenticated" => {
                let session_id = body
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::internal("sciotte authenticated response missing session_id")
                    })?
                    .to_owned();
                let provider = body
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::internal("sciotte authenticated response missing provider")
                    })?
                    .to_owned();
                Ok(RemoteLoginOutcome::Authenticated {
                    session_id,
                    provider,
                })
            }
            "otp_required" => Ok(RemoteLoginOutcome::OtpRequired { flow_id }),
            "two_factor_choice" => Ok(RemoteLoginOutcome::TwoFactorChoice {
                options: body.get("options").cloned().unwrap_or(Value::Null),
                flow_id,
            }),
            "number_match" => Ok(RemoteLoginOutcome::NumberMatch {
                number: body
                    .get("number")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                flow_id,
            }),
            // Only an explicit "failed" is a credential rejection.
            "failed" => Ok(RemoteLoginOutcome::Failed(
                body.get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("login rejected")
                    .to_owned(),
            )),
            // No recognizable login status → a transport/auth/server fault, NOT a
            // login rejection. Surfacing it as an error (rather than Failed) keeps
            // a misconfigured DRAVR_SCIOTTE_API_KEY (401) from masquerading as bad
            // credentials — which is exactly what bit the first e2e run.
            _ => Err(AppError::internal(format!(
                "sciotte returned HTTP {http_status} with no recognizable login status \
                 (body: {body}); check DRAVR_SCIOTTE_API_KEY / service health"
            ))),
        }
    }
}
