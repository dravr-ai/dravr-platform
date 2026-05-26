// ABOUTME: Configurable allowlist of AG-UI event kinds forwarded by an emitter
// ABOUTME: Default allows every kind; operators narrow via with/without/only to reduce noise
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! AG-UI event filter configuration.
//!
//! Emission is configurable so operators can opt out of high-volume streams
//! (e.g. `TEXT_MESSAGE_CONTENT` token deltas on a Telegram channel where
//! per-token updates cost more than they inform) without changing the
//! producer code. Each [`super::emitter::AgUiSink`] carries an
//! [`AgUiEventFilter`] that is consulted before forwarding.

use super::events::AgUiEventKind;
use std::collections::HashSet;

/// Allowlist of event kinds that the emitter is permitted to forward.
///
/// Default allows every kind. Narrow via [`Self::only`], [`Self::with`],
/// and [`Self::without`]; the `Default` impl is intentionally permissive
/// so callers opt into the full AG-UI stream by default and only reduce
/// noise when a specific sink cannot keep up.
#[derive(Debug, Clone)]
pub struct AgUiEventFilter {
    allowed: HashSet<AgUiEventKind>,
}

impl AgUiEventFilter {
    /// Filter that allows every event kind.
    #[must_use]
    pub fn allow_all() -> Self {
        Self {
            allowed: AgUiEventKind::ALL.iter().copied().collect(),
        }
    }

    /// Filter that allows no events.
    #[must_use]
    pub fn deny_all() -> Self {
        Self {
            allowed: HashSet::new(),
        }
    }

    /// Filter that allows only the provided kinds.
    #[must_use]
    pub fn only<I: IntoIterator<Item = AgUiEventKind>>(kinds: I) -> Self {
        Self {
            allowed: kinds.into_iter().collect(),
        }
    }

    /// Add a kind to the allowlist, returning the updated filter.
    #[must_use]
    pub fn with(mut self, kind: AgUiEventKind) -> Self {
        self.allowed.insert(kind);
        self
    }

    /// Remove a kind from the allowlist, returning the updated filter.
    #[must_use]
    pub fn without(mut self, kind: AgUiEventKind) -> Self {
        self.allowed.remove(&kind);
        self
    }

    /// `true` when `kind` is permitted by this filter.
    #[must_use]
    pub fn allows(&self, kind: AgUiEventKind) -> bool {
        self.allowed.contains(&kind)
    }
}

impl Default for AgUiEventFilter {
    fn default() -> Self {
        Self::allow_all()
    }
}
