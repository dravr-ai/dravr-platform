// ABOUTME: Server lifecycle notify events (server.started / server.stopping)
// ABOUTME: Keeps Cloud Run revision/commit env plumbing out of the binary's main()
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use tracing::info;

/// Cloud Run injects the serving revision name here; absent when run locally.
const K_REVISION_ENV: &str = "K_REVISION";

/// Resolve the Cloud Run revision, or `local` when not running on Cloud Run.
fn revision() -> String {
    env::var(K_REVISION_ENV).unwrap_or_else(|_| "local".to_owned())
}

/// Resolve the deployment environment label.
fn environment() -> String {
    env::var("ENVIRONMENT").unwrap_or_else(|_| "unknown".to_owned())
}

/// Raise `server.started` once the process is up and serving.
///
/// Routed by dravr-contremaitre to the deploys channel. This carries no
/// tenant or user scope — it is emitted from the binary's startup path,
/// outside any request.
pub fn notify_started() {
    info!(
        target: "notify",
        event = "server.started",
        revision = %revision(),
        environment = %environment(),
        commit_sha = %env::var("GIT_COMMIT_SHA").unwrap_or_else(|_| "unknown".to_owned()),
        "server started"
    );
}

/// Raise `server.stopping` on SIGTERM.
///
/// Best-effort: the post is fire-and-forget and races Cloud Run's ~10s
/// SIGTERM grace window, so it may be lost. The reliable "is an instance
/// stuck up?" signal is the idle-floor Cloud Run alert, not this event.
pub fn notify_stopping() {
    info!(
        target: "notify",
        event = "server.stopping",
        revision = %revision(),
        environment = %environment(),
        "SIGTERM received"
    );
}
