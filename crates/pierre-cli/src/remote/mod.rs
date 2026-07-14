// ABOUTME: Remote-server client layer for pierre-cli's gcloud-style authenticated admin commands
// ABOUTME: Credential cache, a thin reqwest wrapper, and the RFC 8628 device-login flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote-server client layer for the authenticated admin CLI.
//!
//! Groups the credential cache ([`CachedCredentials`]), the HTTP client
//! ([`RemoteClient`]), and the RFC 8628 device-login flow ([`device_flow`]) that
//! `pierre-cli auth login` and the remote admin subcommands are built on.

pub mod client;
pub mod credentials;
pub mod device_flow;

pub use client::RemoteClient;
pub use credentials::CachedCredentials;
