// ABOUTME: AG-UI emitter trait with registry-backed broadcast sink and a no-op default
// ABOUTME: Pipeline code emits through `AgUiSink`; sinks filter events and fan out to subscribers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! AG-UI emitter trait and implementations.
//!
//! The pipeline emits through the [`AgUiSink`] trait so the producer side
//! stays decoupled from transport. Two implementations ship out of the
//! box:
//!
//! - [`NoopSink`] — discards every event. Used by default when a caller
//!   does not wire AG-UI feedback, so emission cost on the hot path is a
//!   single virtual call and a filter check.
//! - [`BroadcastSink`] — serializes events to JSON and publishes them to
//!   a [`super::registry::RunRegistry`] channel keyed by `run_id`. SSE
//!   route handlers subscribe to the same registry and forward to HTTP
//!   clients.

use super::events::AgUiEvent;
use super::filter::AgUiEventFilter;
use super::registry::{PublishOutcome, RunRegistry};
use async_trait::async_trait;
use tracing::warn;

/// Sink for [`AgUiEvent`]s.
///
/// Implementations MUST consult [`Self::filter`] and drop disallowed
/// events before forwarding. Emission happens on the pipeline's hot
/// path; implementations must be non-blocking and swallow transport
/// errors rather than surfacing them to the producer.
#[async_trait]
pub trait AgUiSink: Send + Sync {
    /// Filter the sink applies before forwarding.
    fn filter(&self) -> &AgUiEventFilter;

    /// Forward an event to the sink. Implementations MUST consult
    /// [`Self::filter`] and drop disallowed events.
    async fn emit(&self, event: &AgUiEvent);
}

/// Sink that discards every event.
///
/// Returned from [`super::PipelineEmitter::none`] and used as the default
/// when AG-UI feedback is not wired for a given turn.
#[derive(Debug, Clone, Default)]
pub struct NoopSink {
    filter: AgUiEventFilter,
}

impl NoopSink {
    /// Construct a no-op sink with the given filter. The filter is
    /// consulted for API compatibility but never actually gates anything
    /// since the sink discards every event regardless.
    #[must_use]
    pub fn new(filter: AgUiEventFilter) -> Self {
        Self { filter }
    }
}

#[async_trait]
impl AgUiSink for NoopSink {
    fn filter(&self) -> &AgUiEventFilter {
        &self.filter
    }

    async fn emit(&self, _event: &AgUiEvent) {}
}

/// Sink that publishes serialized events to a [`RunRegistry`] channel.
///
/// Events are JSON-serialized once in the producer task and broadcast to
/// every subscriber. Serialization errors are logged and dropped so the
/// pipeline never fails a turn on an AG-UI encoding bug.
#[derive(Clone)]
pub struct BroadcastSink {
    registry: RunRegistry,
    filter: AgUiEventFilter,
}

impl BroadcastSink {
    /// Construct a sink that fans events out through `registry` with
    /// `filter` deciding which kinds forward.
    #[must_use]
    pub fn new(registry: RunRegistry, filter: AgUiEventFilter) -> Self {
        Self { registry, filter }
    }
}

#[async_trait]
impl AgUiSink for BroadcastSink {
    fn filter(&self) -> &AgUiEventFilter {
        &self.filter
    }

    async fn emit(&self, event: &AgUiEvent) {
        if !self.filter.allows(event.kind()) {
            return;
        }
        match serde_json::to_string(event) {
            Ok(payload) => match self.registry.publish(event.run_id(), payload) {
                // Delivered to live subscribers, or buffered for
                // future ones — both are normal and silent.
                PublishOutcome::Delivered(_) | PublishOutcome::NoSubscribers => {}
                // Producer published to an unregistered run. This
                // means the channel adapter forgot to call
                // `register` (or unregistered too early), or the
                // producer typoed the `run_id`. Loud warning so
                // operators can spot the bug.
                PublishOutcome::NoSlot => {
                    warn!(
                        run_id = %event.run_id(),
                        kind = ?event.kind(),
                        "AG-UI emit dropped: run_id is not registered"
                    );
                }
            },
            Err(e) => {
                warn!(
                    run_id = %event.run_id(),
                    error = %e,
                    "failed to serialize AG-UI event; dropping"
                );
            }
        }
    }
}
