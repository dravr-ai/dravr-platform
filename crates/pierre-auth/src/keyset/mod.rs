// ABOUTME: The keyset: data and index key families, their lifecycle document, the per-process loader and the rotation controller
// ABOUTME: Implements ADR-017 Phase 4 (carnet#1347): rotation re-seals old rows, and drained keys are retired

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Key rotation
//!
//! - [`document`]: the keyset document, its states and transitions.
//! - [`loader`]: keeps one process's keyring current, every
//!   [`RELOAD_PERIOD`] on every process.
//! - [`controller`]: the background job that promotes, sweeps, disables and
//!   schedules, every [`CONTROLLER_PERIOD`] on one instance.
//! - [`operator`]: the `pierre-cli key` steps: adopt, rotate, enable,
//!   destroy, status.
//! - [`secrets`]: where each version's wrapped key is stored.

use std::time::Duration;

/// The rotation controller and its sweep
pub mod controller;
/// The keyset document, its states and transitions
pub mod document;
/// The per-process keyring loader
pub mod loader;
/// The operator steps behind `pierre-cli key`
pub mod operator;
/// Where each key version's wrapped bytes are stored
pub mod secrets;

pub use controller::{CiphertextHolder, KeysetController, SweepState, TickReport};
pub use document::{KeyState, KeysetDocument, Transition, KEYSET_SECRET};
pub use loader::{read_keyset, KeysetLoader};

/// How often every process re-reads the keyset.
pub const RELOAD_PERIOD: Duration = Duration::from_mins(1);

/// How often the rotation controller ticks.
pub const CONTROLLER_PERIOD: Duration = Duration::from_mins(5);

/// The name the controller's ledger row and log lines carry.
pub const CONTROLLER_WORKER: &str = "key rotation controller";

/// The `key_lifecycle.*` events: the keyset's normal operation. They stay
/// outside the paging `key_material.*` namespace, so a rotation never pages.
pub mod events {
    /// A key was staged.
    pub const STAGED: &str = "key_lifecycle.staged";
    /// A staged key became primary.
    pub const PROMOTED: &str = "key_lifecycle.promoted";
    /// A primary became `decrypt_only`, or a disabled key was enabled back.
    pub const DEMOTED: &str = "key_lifecycle.demoted";
    /// A drained key was disabled.
    pub const DISABLED: &str = "key_lifecycle.disabled";
    /// An operator destroyed a key.
    pub const DESTROYED: &str = "key_lifecycle.destroyed";
    /// A process installed a new keyring.
    pub const KEYRING_RELOADED: &str = "key_lifecycle.keyring_reloaded";
    /// The sweep re-sealed rows under the data primary.
    pub const SWEPT: &str = "key_lifecycle.swept";
    /// A drain count ran for a `decrypt_only` key.
    pub const DRAIN_COUNTED: &str = "key_lifecycle.drain_counted";
    /// The controller staged a scheduled rotation.
    pub const SCHEDULED: &str = "key_lifecycle.scheduled";
    /// The legacy layout was adopted into a keyset.
    pub const ADOPTED: &str = "key_lifecycle.adopted";

    /// Every lifecycle event.
    pub const ALL: [&str; 10] = [
        STAGED,
        PROMOTED,
        DEMOTED,
        DISABLED,
        DESTROYED,
        KEYRING_RELOADED,
        SWEPT,
        DRAIN_COUNTED,
        SCHEDULED,
        ADOPTED,
    ];
}

#[cfg(test)]
mod tests {
    use pierre_database::backends::shared::key_material::{
        DRAIN_COUNT_FAILED, KEYRING_STALE, KEYSET_RELOAD_FAILED, SWEEP_ROW_UNREADABLE,
    };

    use super::events;

    #[test]
    fn lifecycle_events_never_page_and_keyset_failures_always_do() {
        for event in events::ALL {
            assert!(event.starts_with("key_lifecycle."), "{event}");
            assert!(!event.starts_with("key_material."), "{event}");
        }
        for event in [
            DRAIN_COUNT_FAILED,
            KEYRING_STALE,
            KEYSET_RELOAD_FAILED,
            SWEEP_ROW_UNREADABLE,
        ] {
            assert!(event.starts_with("key_material."), "{event}");
        }
    }
}
