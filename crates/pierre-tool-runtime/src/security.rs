// ABOUTME: Guardian security classification — the blast-radius axis every tool declares for the dispatch-time guard.
// ABOUTME: SecurityLabels bitflags + the RuntimeTool supertrait that forces every registered tool to classify itself.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Guardian security labels
//!
//! The [`Guardian`](crate::guardian::Guardian) gates tool dispatch on a
//! security axis that is **orthogonal** to [`ToolCapabilities`](crate::ToolCapabilities)
//! (auth/tenant/admin/read/write) and to the registry's string categories
//! (domain taxonomy). `WRITES_DATA` is not "irreversible" (`set_goal` is a
//! reversible internal write), so this axis needs its own flags.
//!
//! Labels are declared per tool via the [`RuntimeTool`] supertrait, which has
//! **no default** `security_class` — a tool will not compile into the registry
//! (whose stored type is `Arc<dyn RuntimeTool>`) until it classifies itself.
//! Drift is therefore unrepresentable rather than test-caught: a new tool that
//! forgets to classify fails to build, it cannot silently ship unguarded.
//!
//! The labels never cross the tronc boundary (they are not part of
//! `capabilities()`), never serialize onto the MCP wire, and never reach the
//! TypeScript SDK — they are server-internal egress policy.

use bitflags::bitflags;

use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::tool::McpTool;
/// Re-exported so [`declare_security!`] can name it through `$crate` in the
/// crates that expand it.
pub use pierre_core::transport::TransportPolicy;

bitflags! {
    /// The Guardian's blast-radius classification for a tool.
    ///
    /// Empty (the common case) = an internal, reversible tool that costs no
    /// destructive budget. Flags are additive.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct SecurityLabels: u8 {
        /// The tool performs an irreversible / destructive effect (deletes,
        /// disconnects). The Guardian caps these per turn.
        const IRREVERSIBLE = 0b0000_0100;
    }
}

impl SecurityLabels {
    /// The tool performs an irreversible / destructive effect.
    #[must_use]
    pub const fn is_irreversible(self) -> bool {
        self.contains(Self::IRREVERSIBLE)
    }
}

/// Platform extension of tronc's [`McpTool`]: every runtime tool also declares
/// its Guardian security posture.
///
/// There is deliberately **no default** `security_class`. The registry stores
/// `Arc<dyn RuntimeTool>`, so a tool that does not implement this trait cannot
/// be registered — the omission is a compile error, not a runtime gap. This is
/// the single source of truth for a tool's egress classification; there is no
/// parallel name-keyed table to keep in sync.
///
/// `dyn RuntimeTool` is object-safe: `McpTool` is, and `security_class` returns
/// a `Copy` value. All `McpTool` methods (`definition`/`capabilities`/`execute`)
/// remain callable on a `dyn RuntimeTool` via the supertrait.
pub trait RuntimeTool: McpTool<dyn ToolRuntime> {
    /// The tool's Guardian security classification (see [`SecurityLabels`]).
    fn security_class(&self) -> SecurityLabels;

    /// Where the tool may be listed and run: on every transport, or only on
    /// Dravr's own surfaces ([`Transport::is_first_party`]).
    ///
    /// The same [`TransportPolicy`] a provider declares for its data, read the
    /// same way: a [`TransportPolicy::FirstPartyOnly`] tool is left out of an
    /// external caller's listing and refused if it calls it anyway
    /// ([`crate::first_party_tools`]). An MCP client, an A2A agent and an API
    /// key are external; the web and mobile apps, linked messaging channels
    /// and platform jobs are not.
    ///
    /// Defaults to [`TransportPolicy::AnyTransport`]: a tool is listed and run
    /// on every transport unless it says otherwise. A tool opts in through
    /// [`declare_security!`](crate::declare_security)'s `first_party_only`
    /// arm, beside its security labels.
    ///
    /// [`Transport::is_first_party`]: pierre_core::transport::Transport::is_first_party
    fn transport_policy(&self) -> TransportPolicy {
        TransportPolicy::AnyTransport
    }

    /// Whether the tool can only fail on a deployment holding no USDA key
    /// ([`crate::deployment_tools`]). A tool opts in through
    /// [`declare_security!`](crate::declare_security)'s `food_database` arm.
    fn requires_food_database(&self) -> bool {
        false
    }
}

/// Implement [`RuntimeTool`] for a tool type in one line, co-located with its
/// `McpTool` impl (and therefore under the same `#[cfg(feature = ...)]`).
///
/// ```ignore
/// declare_security!(GetActivitiesTool => empty);
/// declare_security!(DeleteAgentTool   => IRREVERSIBLE);
/// declare_security!(SetGoalTool       => empty);
/// declare_security!(GetRosterOverviewTool => empty; first_party_only);
/// ```
///
/// The trailing `; first_party_only` also declares the tool
/// [`TransportPolicy::FirstPartyOnly`] ([`RuntimeTool::transport_policy`]):
/// listed and run on Dravr's own surfaces only. `; food_database` declares
/// that the tool needs the deployment's USDA key
/// ([`RuntimeTool::requires_food_database`]).
///
/// `#[macro_export]` so the three pierre-server endurance tools (and test
/// fakes) can classify themselves too — `$crate` resolves to
/// `pierre_tool_runtime` regardless of the call site.
#[macro_export]
macro_rules! declare_security {
    ($t:ty => empty $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }
        }
    };
    ($t:ty => empty ; food_database $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }

            fn requires_food_database(&self) -> bool {
                true
            }
        }
    };
    ($t:ty => empty ; first_party_only $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }

            fn transport_policy(&self) -> $crate::security::TransportPolicy {
                $crate::security::TransportPolicy::FirstPartyOnly
            }
        }
    };
    ($t:ty => $($flag:ident)|+ ; first_party_only $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn security_class(&self) -> $crate::security::SecurityLabels {
                $( $crate::security::SecurityLabels::$flag )|+
            }

            fn transport_policy(&self) -> $crate::security::TransportPolicy {
                $crate::security::TransportPolicy::FirstPartyOnly
            }
        }
    };
    ($t:ty => $($flag:ident)|+ $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn security_class(&self) -> $crate::security::SecurityLabels {
                $( $crate::security::SecurityLabels::$flag )|+
            }
        }
    };
}
