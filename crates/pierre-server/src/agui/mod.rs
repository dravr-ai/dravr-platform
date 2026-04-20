// ABOUTME: AG-UI protocol integration — event schema, filter, emitter, registry, SSE route
// ABOUTME: Implements https://github.com/ag-ui-protocol/ag-ui for streaming agent progress to clients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # AG-UI (Agent-User Interaction) integration
//!
//! Implements the [AG-UI protocol](https://github.com/ag-ui-protocol/ag-ui)
//! event schema and SSE transport so web, mobile, and messaging clients
//! can receive live progress feedback while the chat pipeline runs an
//! agent turn.
//!
//! ## Components
//!
//! - [`events`] — the AG-UI event enum and kind discriminant. Serializes
//!   to the AG-UI wire format (JSON objects tagged by `type`) so
//!   off-the-shelf AG-UI clients interop.
//! - [`filter`] — [`AgUiEventFilter`] configuration that narrows the set
//!   of event kinds a sink forwards. Operators can opt out of
//!   high-volume streams like `TEXT_MESSAGE_CONTENT` without changing
//!   the producer code.
//! - [`registry`] — [`RunRegistry`] maps `run_id` to a tokio broadcast
//!   channel; the pipeline publishes and SSE subscribers consume.
//! - [`emitter`] — [`AgUiSink`] trait with a [`NoopSink`] default and a
//!   registry-backed [`BroadcastSink`] for production use.
//! - [`routes`] — [`AgUiRoutes`] exposes
//!   `GET /api/agui/runs/{run_id}/stream` as an SSE endpoint.
//!
//! ## Pipeline wiring
//!
//! The chat pipeline ([`crate::services::chat_pipeline`]) threads an
//! optional `&dyn AgUiSink` through its hooks. Channel adapters that
//! care about progress feedback construct a [`BroadcastSink`] bound to
//! a fresh `run_id`, pass it in, and hand the `run_id` back to the
//! client so it can open the SSE stream. The sink is silently skipped
//! when absent, so the feature is strictly opt-in per turn.

pub mod emitter;
pub mod events;
pub mod filter;
pub mod registry;

#[cfg(feature = "transport-sse")]
pub mod routes;

pub use emitter::{AgUiSink, BroadcastSink, NoopSink};
pub use events::{AgUiEvent, AgUiEventKind};
pub use filter::AgUiEventFilter;
pub use registry::{
    AuthorizedSubscribe, PublishOutcome, RunOwner, RunRegistry, RunScope, RunSubscription,
};

#[cfg(feature = "transport-sse")]
pub use routes::AgUiRoutes;
