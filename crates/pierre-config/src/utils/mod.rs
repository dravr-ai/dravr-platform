// ABOUTME: Server-coupled utility modules wired against ServerConfig statics
// ABOUTME: Hosts http_client and route_timeout helpers; leaf utilities live in pierre-core
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// HTTP client configuration and helpers
pub mod http_client;
/// Route timeout configuration and middleware
pub mod route_timeout;
