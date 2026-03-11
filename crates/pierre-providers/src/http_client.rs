// ABOUTME: Re-exports the shared HTTP client from pierre-core for provider API calls
// ABOUTME: Thin wrapper preserving the public API while delegating to the centralized client
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_core::http_client::api_client as shared_client;
pub use pierre_core::http_client::initialize_api_client as initialize_shared_client;
