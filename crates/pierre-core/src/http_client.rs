// ABOUTME: Shared HTTP client singletons with connection pooling for all outbound requests
// ABOUTME: Provides api_client (30s) and llm_client (300s) to eliminate duplicate client creation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use reqwest::{Client, ClientBuilder};
use std::env;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::error;

/// Default request timeout for API calls (30 seconds)
const DEFAULT_API_TIMEOUT_SECS: u64 = 30;

/// Default connect timeout for API calls (10 seconds)
const DEFAULT_API_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Default connect timeout for LLM API calls (30 seconds)
const DEFAULT_LLM_CONNECT_TIMEOUT_SECS: u64 = 30;

/// Default request timeout for LLM API calls (5 minutes, completions can be slow)
const DEFAULT_LLM_REQUEST_TIMEOUT_SECS: u64 = 300;

/// Configured timeout overrides for the API client
static API_CLIENT_TIMEOUTS: OnceLock<(u64, u64)> = OnceLock::new();

/// Global shared HTTP client for data-provider API calls (Strava, Garmin, etc.)
static API_CLIENT: OnceLock<Client> = OnceLock::new();

/// Global shared HTTP client for LLM API calls (Gemini, Groq, `OpenAI`, etc.)
static LLM_CLIENT: OnceLock<Client> = OnceLock::new();

/// Initialize the API client timeout configuration
///
/// Call once at server startup before any providers are created.
/// If not called, defaults are used (30s request, 10s connect).
pub fn initialize_api_client(timeout_secs: u64, connect_timeout_secs: u64) {
    let _ = API_CLIENT_TIMEOUTS.set((timeout_secs, connect_timeout_secs));
}

/// Get the shared HTTP client for data-provider API calls
///
/// Uses connection pooling with configured timeouts (default: 30s request, 10s connect).
/// Prefer this over creating new clients for external data APIs.
pub fn api_client() -> &'static Client {
    API_CLIENT.get_or_init(|| {
        let (timeout, connect_timeout) = API_CLIENT_TIMEOUTS
            .get()
            .copied()
            .unwrap_or((DEFAULT_API_TIMEOUT_SECS, DEFAULT_API_CONNECT_TIMEOUT_SECS));

        build_client(timeout, connect_timeout, "api")
    })
}

/// Get the shared HTTP client for LLM API calls
///
/// Uses connection pooling with longer timeouts (default: 300s request, 30s connect)
/// suitable for LLM completion requests that can take minutes.
///
/// Timeouts are configurable via environment variables:
/// - `LLM_CONNECT_TIMEOUT_SECS` (default: 30)
/// - `LLM_REQUEST_TIMEOUT_SECS` (default: 300)
pub fn llm_client() -> &'static Client {
    LLM_CLIENT.get_or_init(|| {
        let connect_secs = env::var("LLM_CONNECT_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_LLM_CONNECT_TIMEOUT_SECS);
        let request_secs = env::var("LLM_REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_LLM_REQUEST_TIMEOUT_SECS);

        build_client(request_secs, connect_secs, "llm")
    })
}

/// Build a reqwest client with the given timeouts, falling back to a default on error
fn build_client(timeout_secs: u64, connect_timeout_secs: u64, label: &str) -> Client {
    ClientBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(connect_timeout_secs))
        .build()
        .unwrap_or_else(|e| {
            error!("Failed to build {label} HTTP client: {e}; falling back to default client");
            Client::new()
        })
}
