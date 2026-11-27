// ABOUTME: The transport a call is served over: declared by each layer, narrowed by every enclosing one
// ABOUTME: serve_over declares it; declared_transport, served_outside and effective_transport read it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The transport a call is served over, as the layers that serve it declare.

use std::future::Future;

use pierre_core::transport::Transport;

/// Run `fut` as a call served over `transport`.
///
/// A declaration only narrows: inside a call already served over an external
/// transport, the work stays external whatever it declares. The enclosing
/// declaration is read when the future runs, not when it is built, so a
/// future built outside a scope and awaited inside it is still narrowed.
pub async fn serve_over<F: Future>(transport: Transport, fut: F) -> F::Output {
    let effective = declared_transport().map_or(transport, |outer| outer.narrowed_by(transport));
    super::TRANSPORT.scope(effective, fut).await
}

/// The transport declared for the current call, if any.
#[must_use]
pub fn declared_transport() -> Option<Transport> {
    super::TRANSPORT.try_with(|transport| *transport).ok()
}

/// Whether the current request is served to an outside client.
///
/// It is when its credential is an outside client's (`outside_credential`:
/// an API key, an MCP token, an OAuth application's grant), or when the
/// transport layer declared it outside Dravr's own surfaces. A request no
/// layer declared is read by its credential alone.
#[must_use]
pub fn served_outside(outside_credential: bool) -> bool {
    outside_credential || declared_transport().is_some_and(|served| !served.is_first_party())
}

/// The transport a call bound to `bound` serves when it runs here: the
/// enclosing declaration narrowed by `bound`, or whichever of the two exists.
#[must_use]
pub fn effective_transport(bound: Option<Transport>) -> Option<Transport> {
    match (declared_transport(), bound) {
        (Some(outer), Some(inner)) => Some(outer.narrowed_by(inner)),
        (outer, inner) => outer.or(inner),
    }
}
