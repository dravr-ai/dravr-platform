// ABOUTME: Audit-trail helpers for the MCP tool-call path
// ABOUTME: Fingerprints call arguments so the trail can identify a call without carrying its contents
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Audit-trail support for `tools/call`.
//!
//! The 2026-07-28 revision asks for an audit event per tool call naming, among
//! other things, the arguments the call was made with. Naming them literally is
//! not available here: tool arguments carry athlete data throughout and, on the
//! connect-style tools, provider credentials, so a log line holding them would
//! breach the logging-hygiene rule the rest of the server follows.
//!
//! The event names the client by its OAuth `client_id` (RFC 9068 §2.2), the
//! identifier assigned at registration and checked when the token was issued,
//! never by the `clientInfo` or `User-Agent` a caller sends about itself.

use dravr_tronc::mcp::tool::ToolContext;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tracing::Span;

use pierre_tool_runtime::schema_canonical::to_canonical_value;

/// Fingerprint a tool call's arguments for the audit trail.
///
/// A digest gives the trail the property it actually needs — two calls with
/// identical arguments share a fingerprint, two different calls do not — while
/// carrying nothing back out of the process.
///
/// Rendered through the canonical serializer so the fingerprint is a function
/// of the *arguments*, not of the order a map happened to iterate in: with
/// `serde_json`'s `preserve_order` enabled, an unsorted render would give the
/// same call a different digest on every request and make the field useless for
/// correlating a replay — while still looking like it worked.
///
/// A `null` argument value hashes like the empty object rather than being
/// skipped, so a no-argument call still gets a stable, comparable fingerprint.
#[must_use]
pub fn arguments_fingerprint(args: &Value) -> String {
    let empty = Value::Object(Map::new());
    let canonical = to_canonical_value(if args.is_null() { &empty } else { args });
    let rendered = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&rendered))
}

/// Record a tool call's identity into the current tracing span.
///
/// Keeps the audit fields and the rule about what may be logged together in
/// one place, rather than spread across the dispatch path: the call site names
/// the call, this decides what is safe to say about it.
///
/// `client` is the OAuth `client_id` the credential was issued to, which the
/// auth hook verified ([`ToolContext::client_id`]) — the client the
/// authorization server registered and checked at issuance. It is left empty
/// for a credential no registered client holds, such as an API key, and is
/// never filled from what the caller says about itself: the request's
/// `_meta` `clientInfo` and its `User-Agent` are the caller's own choice, and
/// an audit trail that recorded them would name whoever the caller claimed.
pub fn record_tool_call(tool_name: &str, args: &Value, ctx: &ToolContext) {
    let span = Span::current();
    span.record("tool_name", tool_name);
    span.record("arguments_hash", arguments_fingerprint(args).as_str());
    if let Some(client_id) = ctx.client_id.as_deref() {
        span.record("client", client_id);
    }
}
