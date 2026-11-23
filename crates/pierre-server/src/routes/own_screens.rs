// ABOUTME: The refusal of an outside credential on a route served to the athlete's own screens only
// ABOUTME: An API key is admitted by REST authentication; these routes turn it away with one 403 (carnet#1543, carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes Dravr serves only on the athlete's own screens.
//!
//! REST authentication already refuses a delegated OAuth grant and a
//! personal MCP token on every route. An API key is admitted, as the
//! athlete's own credential, so a route that serves only the athlete's own
//! screens (their health records, who read their data) refuses it here,
//! as it would any credential served outside Dravr's own surfaces.

use pierre_auth::auth::AuthResult;
use pierre_providers::ai_scope;

/// Whether the request `auth` carries is served to an outside client: an
/// outside credential, or a request the transport layer declared outside
/// Dravr's own surfaces. Each route refuses it with its own literal
/// `PermissionDenied` sentence, so the refusal review reads every one.
pub(crate) fn served_to_outside_client(auth: &AuthResult) -> bool {
    ai_scope::served_outside(auth.acts_for_outside_client())
}
