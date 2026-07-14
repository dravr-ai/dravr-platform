// ABOUTME: Minimal authenticated HTTP client for pierre-cli's remote admin commands
// ABOUTME: Wraps reqwest with the server base URL and an optional cached bearer token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Thin HTTP client used by the remote admin subcommands.
//!
//! Wraps `reqwest` so each command is a couple of lines: build from cached
//! credentials, call a verb, get back parsed JSON or an actionable error. The
//! device-login flow needs the raw response (to read RFC 8628 `400` bodies), so
//! [`RemoteClient::post_raw`] is exposed alongside the 2xx-or-error helpers.

use std::time::Duration;

use pierre_core::errors::{AppError, AppResult};
use serde::Serialize;
use serde_json::{json, Value};

use super::credentials::CachedCredentials;

/// Default per-request timeout. The device-token poll uses this per attempt.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// An HTTP client bound to one pierre-server, optionally carrying a bearer token.
pub struct RemoteClient {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl RemoteClient {
    /// Build a client for `base` (trailing slash trimmed), optionally authenticated.
    ///
    /// # Errors
    /// Returns an error if the underlying `reqwest` client cannot be built.
    pub fn new(base: &str, token: Option<String>) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("pierre-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::internal(format!("Failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            token,
        })
    }

    /// Build an authenticated client from cached login credentials.
    ///
    /// # Errors
    /// Returns an error if the client cannot be built.
    pub fn from_cached(creds: &CachedCredentials) -> AppResult<Self> {
        Self::new(&creds.server, Some(creds.access_token.clone()))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    /// POST JSON and return the raw response so the caller can inspect status.
    ///
    /// Used by the device-token poll, which treats a `400 { "error": ... }` as a
    /// protocol signal (`authorization_pending`), not a transport failure.
    ///
    /// # Errors
    /// Returns an error only when the request cannot be sent.
    pub async fn post_raw<B: Serialize + Sync>(
        &self,
        path: &str,
        body: &B,
    ) -> AppResult<reqwest::Response> {
        self.authed(self.http.post(self.url(path)).json(body))
            .send()
            .await
            .map_err(|e| Self::transport_error(path, &e))
    }

    /// POST JSON, requiring a 2xx; returns the parsed body.
    ///
    /// # Errors
    /// Returns an error on transport failure or a non-2xx status.
    pub async fn post_json<B: Serialize + Sync>(&self, path: &str, body: &B) -> AppResult<Value> {
        let resp = self.post_raw(path, body).await?;
        Self::json_or_error(path, resp).await
    }

    /// GET, requiring a 2xx; returns the parsed body.
    ///
    /// # Errors
    /// Returns an error on transport failure or a non-2xx status.
    pub async fn get_json(&self, path: &str) -> AppResult<Value> {
        let resp = self
            .authed(self.http.get(self.url(path)))
            .send()
            .await
            .map_err(|e| Self::transport_error(path, &e))?;
        Self::json_or_error(path, resp).await
    }

    /// PATCH JSON, requiring a 2xx; returns the parsed body.
    ///
    /// # Errors
    /// Returns an error on transport failure or a non-2xx status.
    pub async fn patch_json<B: Serialize + Sync>(&self, path: &str, body: &B) -> AppResult<Value> {
        let resp = self
            .authed(self.http.patch(self.url(path)).json(body))
            .send()
            .await
            .map_err(|e| Self::transport_error(path, &e))?;
        Self::json_or_error(path, resp).await
    }

    /// DELETE, requiring a 2xx; returns the parsed body.
    ///
    /// # Errors
    /// Returns an error on transport failure or a non-2xx status.
    pub async fn delete_json(&self, path: &str) -> AppResult<Value> {
        let resp = self
            .authed(self.http.delete(self.url(path)))
            .send()
            .await
            .map_err(|e| Self::transport_error(path, &e))?;
        Self::json_or_error(path, resp).await
    }

    fn transport_error(path: &str, err: &reqwest::Error) -> AppError {
        AppError::external_service("pierre-server", format!("request to {path} failed: {err}"))
    }

    /// Parse a 2xx JSON body, or map a non-2xx to an actionable error. Reads the
    /// server's `AdminResponse.message` when present so the operator sees why.
    async fn json_or_error(path: &str, resp: reqwest::Response) -> AppResult<Value> {
        let status = resp.status();
        let body = resp.json::<Value>().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let message = body
            .get("message")
            .or_else(|| body.get("error_description"))
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_owned();
        match status.as_u16() {
            401 => Err(AppError::auth_invalid(format!(
                "Not authorized calling {path}. Run `pierre-cli auth login`. ({message})"
            ))),
            403 => Err(AppError::auth_invalid(format!(
                "Forbidden calling {path}: {message} (a super-admin login is required)"
            ))),
            _ => Err(AppError::external_service(
                "pierre-server",
                format!("{status} from {path}: {message}"),
            )),
        }
    }
}

/// Convenience: an empty JSON object body for verb calls that take no payload.
#[must_use]
pub fn empty_body() -> Value {
    json!({})
}
