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
use crate::tool_data::ToolData;
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities};

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

    /// What the tool reads ([`ToolData`]): whose data a coach's call may read
    /// through it, the streams its result serves, and whether written text
    /// passes through it. No default, as for [`Self::security_class`]: a tool
    /// that does not declare it does not compile. The registry reads it once,
    /// at registration. A tool declared under no class ([`ToolData::own`])
    /// reads only its caller's own data, if any, and a coach's thread about
    /// an athlete withholds it.
    fn tool_data(&self) -> ToolData;

    /// Where the tool may be listed and run ([`ToolReach`]): on every
    /// transport, only on Dravr's own surfaces, or only in Dravr's own app
    /// whoever asks.
    ///
    /// Read by the one access predicate ([`crate::tool_access::withheld`]):
    /// a [`ToolReach::FirstPartyOnly`] tool is left out of an external
    /// caller's listing and refused if it calls it anyway, unless the caller
    /// is an admin whose grant holds `admin`; a [`ToolReach::AppOnly`] tool
    /// is withheld from every external caller. An MCP client, an A2A agent
    /// and an API key are external; the web and mobile apps, linked
    /// messaging channels and platform jobs are not.
    ///
    /// Defaults to [`ToolReach::AnyTransport`]. A tool opts in through
    /// [`declare_security!`](crate::declare_security)'s `first_party_only` or
    /// `app_only` arm, beside its security labels.
    fn reach(&self) -> ToolReach {
        ToolReach::AnyTransport
    }

    /// Who the tool serves ([`Audience`]), the role axis of
    /// [`crate::tool_access::withheld`].
    ///
    /// Derived, never listed: an `ADMIN_ONLY` tool serves admins, a tool
    /// declared through [`declare_security!`](crate::declare_security)'s
    /// `coach` arm serves a human coach (and an admin), every other tool
    /// serves everyone.
    fn audience(&self) -> Audience {
        if self.capabilities().contains(ToolCapabilities::ADMIN_ONLY) {
            Audience::Admin
        } else {
            Audience::Athlete
        }
    }

    /// Whether the tool carries the user's whole record away (an export), so
    /// that an admin's support session is withheld it
    /// ([`crate::tool_access::withheld`]): support may see what it needs to
    /// help, never take the whole record with it. A tool opts in through
    /// [`declare_security!`](crate::declare_security)'s `export` arm.
    fn exports_record(&self) -> bool {
        false
    }

    /// Whether the tool can only fail on a deployment holding no USDA key
    /// ([`crate::deployment_tools`]). A tool opts in through
    /// [`declare_security!`](crate::declare_security)'s `food_database` arm.
    fn requires_food_database(&self) -> bool {
        false
    }
}

/// Where a tool may be listed and run, by the transport a call is served
/// over and, for one value, the caller's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolReach {
    /// Every transport.
    #[default]
    AnyTransport,
    /// Dravr's own surfaces, and an admin's external connection whose grant
    /// holds `admin` (carnet#1511, Q14 and Q25).
    FirstPartyOnly,
    /// Dravr's own surfaces only, whoever asks: every effect the tool has is
    /// one an external transport refuses for everyone (carnet#1533), so
    /// listing it externally would offer a call that can only be refused.
    AppOnly,
}

/// Who a tool serves: the role axis of the access predicate
/// ([`crate::tool_access`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Everyone who signs in: the athlete tools.
    Athlete,
    /// A human coach who leads an active group, and an admin: the roster
    /// tools over other members' data. On Dravr's own surfaces the tool's own
    /// gates decide (a peer reads a group member there too).
    Coach,
    /// An admin: the `ADMIN_ONLY` tools.
    Admin,
}

/// Implement [`RuntimeTool`] for a tool type, co-located with its `McpTool`
/// impl (and therefore under the same `#[cfg(feature = ...)]`).
///
/// ```ignore
/// declare_security!(
///     GetActivitiesTool => empty,
///     reads: ToolData::of(DataClass::Training, &[Stream::Workout]),
/// );
/// declare_security!(DeleteAgentTool => IRREVERSIBLE, reads: ToolData::own(&[]));
/// declare_security!(GetRosterOverviewTool => empty; coach, reads: ToolData::own(&[]));
/// declare_security!(RememberFactTool => empty; first_party_only, reads: ToolData::own(&[]));
/// ```
///
/// `reads:` is the tool's [`ToolData`] ([`RuntimeTool::tool_data`]): what it
/// reads, required on every arm.
///
/// The trailing `; first_party_only` declares the tool
/// [`ToolReach::FirstPartyOnly`] ([`RuntimeTool::reach`]): listed and run
/// on Dravr's own surfaces, and to an admin holding `admin` in their grant.
/// `; app_only` declares [`ToolReach::AppOnly`]: Dravr's own surfaces
/// only, whoever asks. `; coach` declares that the tool serves a human coach
/// ([`Audience::Coach`], [`RuntimeTool::audience`]). `; food_database`
/// declares that the tool needs the deployment's USDA key
/// ([`RuntimeTool::requires_food_database`]). `; export` declares that the
/// tool carries the user's whole record away ([`RuntimeTool::exports_record`]).
///
/// `#[macro_export]` so the three pierre-server endurance tools (and test
/// fakes) can classify themselves too — `$crate` resolves to
/// `pierre_tool_runtime` regardless of the call site.
#[macro_export]
macro_rules! declare_security {
    ($t:ty => empty , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }
        }
    };
    ($t:ty => empty ; food_database , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }

            fn requires_food_database(&self) -> bool {
                true
            }
        }
    };
    ($t:ty => empty ; export , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }

            fn exports_record(&self) -> bool {
                true
            }
        }
    };
    ($t:ty => empty ; first_party_only , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }

            fn reach(&self) -> $crate::security::ToolReach {
                $crate::security::ToolReach::FirstPartyOnly
            }
        }
    };
    ($t:ty => $($flag:ident)|+ ; first_party_only , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $( $crate::security::SecurityLabels::$flag )|+
            }

            fn reach(&self) -> $crate::security::ToolReach {
                $crate::security::ToolReach::FirstPartyOnly
            }
        }
    };
    ($t:ty => empty ; coach , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $crate::security::SecurityLabels::empty()
            }

            fn audience(&self) -> $crate::security::Audience {
                $crate::security::Audience::Coach
            }
        }
    };
    ($t:ty => $($flag:ident)|+ ; app_only , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $( $crate::security::SecurityLabels::$flag )|+
            }

            fn reach(&self) -> $crate::security::ToolReach {
                $crate::security::ToolReach::AppOnly
            }
        }
    };
    ($t:ty => $($flag:ident)|+ , reads: $data:expr $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn tool_data(&self) -> $crate::tool_data::ToolData {
                $data
            }

            fn security_class(&self) -> $crate::security::SecurityLabels {
                $( $crate::security::SecurityLabels::$flag )|+
            }
        }
    };
}
