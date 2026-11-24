// ABOUTME: Admin health check route handler
// ABOUTME: Answers GET /admin/health with the admin API's status and version
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::Json;
use dravr_tronc::dates::Timestamp;
use serde_json::json;
use tokio::task;

/// Handle health check (GET /admin/health)
pub(crate) async fn handle_health() -> Json<serde_json::Value> {
    let health_json = task::spawn_blocking(|| {
        json!({
            "status": "healthy",
            "service": "pierre-mcp-admin-api",
            "timestamp": Timestamp::now().to_rfc3339(),
            "version": env!("CARGO_PKG_VERSION")
        })
    })
    .await
    .unwrap_or_else(|_| {
        json!({
            "status": "error",
            "service": "pierre-mcp-admin-api"
        })
    });

    Json(health_json)
}
