// ABOUTME: Identity route group — OAuth 2.0 authorization server + per-user OAuth-app management
// ABOUTME: Generic over IdentityCtx (+ MiddlewareCtx for AuthenticatedUser) so the crate stays decoupled from pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Identity Routes
//!
//! Hosts two related but independently mounted route groups:
//!
//! - **OAuth 2.0 authorization server** (`/.well-known/{oauth-authorization-server,
//!   jwks.json}`, `/oauth2/{register,authorize,token,login,…}`) — the in-tree
//!   authorization-server endpoints that the `pierre_auth::oauth2_server`
//!   state machine sits behind. Public endpoints (`/oauth2/register`,
//!   `/oauth2/token`) are rate-limited per client IP.
//! - **Per-user OAuth-app management** (`/api/users/oauth-apps`) — endpoints
//!   that let authenticated users register their own per-provider OAuth
//!   client credentials (Strava, Fitbit, Garmin, …) to avoid the shared
//!   tenant rate limits.
//!
//! Both groups are generic over [`pierre_runtime_context::IdentityCtx`].
//! The user-OAuth-app endpoints additionally need
//! [`pierre_runtime_context::MiddlewareCtx`] for the `AuthenticatedUser`
//! extractor; the composition root in `pierre-server` implements both
//! traits on its `ServerContext`.
//!
//! Note: the broader auth surface (credential login, OAuth callback
//! handling for fitness providers, Sciotte hosted-login flow,
//! provider-link webhook) is still tangled with `pierre-server`-internal
//! types (`AuthService`, `OAuthService`,
//! `crate::context::{AuthContext, ConfigContext, DataContext}`) and stays
//! in `pierre-server` for now. (`FirebaseAuth` itself now lives in
//! `pierre_auth::firebase`.)

#![warn(missing_docs)]

/// OAuth 2.0 authorization-server endpoints (RFC 6749 / 8414 / 7591 / 7636).
pub mod oauth2;

/// Per-user OAuth-app credential management endpoints.
pub mod user_oauth_apps;

pub use oauth2::{LoginHtmlParams, OAuth2Routes};
pub use user_oauth_apps::UserOAuthAppRoutes;
