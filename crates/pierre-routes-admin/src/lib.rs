// ABOUTME: Admin route group — programmatic /admin/* (admin-token JWT) + cookie-auth /api/admin/*
// ABOUTME: Owns AdminApiContext, AdminRoutes, AdminAuthService, AdminJwtManager — decoupled from pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Admin Routes
//!
//! Hosts every admin REST surface that ships with the platform:
//!
//! - Programmatic admin-token JWT routes mounted at `/admin/...`
//!   ([`AdminRoutes::routes`]) — API key provisioning, admin token
//!   mint/rotate, user lifecycle, system settings, store review, initial
//!   setup. Consumed by `pierre-cli` and B2B partners.
//!
//! - Human admin cookie-auth routes mounted at `/api/admin/...`
//!   ([`AdminRoutes::cookie_admin_routes`]) — claim verdicts, memory worker
//!   metrics, coach followup triage, coach notes audit, myth-busting,
//!   coach grading, harness configuration. Powers the admin web UI tabs.
//!
//! The cookie-auth surface gates on
//! [`pierre_middleware::cookie_admin_middleware`], itself generic over
//! [`pierre_runtime_context::MiddlewareCtx`]; the composition root in
//! `pierre-server` implements the trait on its `ServerContext` and passes
//! `Arc<ServerContext>` as the layer state.
//!
//! Admin token JWT validation lives in [`auth::AdminAuthService`] /
//! [`auth::jwt::AdminJwtManager`]; these are re-exported from
//! `pierre-server`'s `admin` module for callers that still use the old
//! path (notably `routes::auth::login::handle_register`).

#![warn(missing_docs)]

/// Admin authentication services (JWT-token and cookie-derived).
pub mod auth;

/// Admin REST handler functions, grouped by sub-area.
pub mod handlers;

/// Per-tenant + per-user LLM provider settings (Gemini, Groq, Local LLM).
pub mod llm_settings;

/// Admin tool-selection routes — per-tenant tool-availability overrides.
pub mod tool_selection;

mod context;
mod routes;

pub use auth::{
    middleware::admin_auth_middleware, service::AdminAuthService, AdminJwtManager,
    TokenGenerationConfig,
};
pub use context::{AdminApiContext, AdminApiContextInit};
pub use handlers::feature_flags::FeatureFlagsRoutes;
pub use handlers::impersonation::ImpersonationRoutes;
pub use handlers::llm_consumption::LlmConsumptionRoutes;
pub use handlers::types::{
    AdminResponse, AdminSetupRequest, AdminSetupResponse, ApproveUserRequest, AutoApprovalResponse,
    DeleteUserRequest, ListApiKeysQuery, ListUsersQuery, ProvisionApiKeyRequest,
    ProvisionApiKeyResponse, RateLimitInfo, RevokeKeyRequest, SuspendUserRequest,
    TenantCreatedInfo, UpdateAutoApprovalRequest, UserActivityQuery,
};
pub use routes::AdminRoutes;
