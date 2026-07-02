// ABOUTME: Guardian security classification — the egress/trust axis every tool declares for the dispatch-time guard.
// ABOUTME: SecurityLabels bitflags + the RuntimeTool supertrait that forces every registered tool to classify itself.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Guardian security labels
//!
//! The [`Guardian`](crate::guardian::Guardian) gates tool dispatch on a
//! security axis that is **orthogonal** to [`ToolCapabilities`](crate::ToolCapabilities)
//! (auth/tenant/admin/read/write) and to the registry's string categories
//! (domain taxonomy). `READS_DATA` is not "produces untrusted content"
//! (reading your own goal is a trusted read) and `WRITES_DATA` is not
//! "irreversible" (`set_goal` is a reversible internal write), so the taint
//! axis needs its own flags.
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

use std::sync::Arc;

use bitflags::bitflags;

use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::tool::McpTool;

bitflags! {
    /// The Guardian's egress/trust classification for a tool.
    ///
    /// Empty (the common case) = a trusted, internal, reversible tool that the
    /// Guardian never gates on taint grounds. Flags are additive.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct SecurityLabels: u8 {
        /// The tool echoes third-party / external text back into the LLM
        /// context (provider scrapes, peer data, external API bodies,
        /// coach-authored notes). This is the prompt-injection *source*: its
        /// output is untrusted data that re-enters the instruction channel.
        const UNTRUSTED_OUTPUT = 0b0000_0001;
        /// The tool sends data outbound to a third party (messaging / email /
        /// push). The exfiltration *sink*. None exist today; the label is armed
        /// for the messaging epic so the guard is in place before the tools are.
        const EXTERNAL_SEND = 0b0000_0010;
        /// The tool performs an irreversible / destructive effect (deletes,
        /// disconnects). The Guardian caps these per turn and can require
        /// confirmation when the turn is tainted.
        const IRREVERSIBLE = 0b0000_0100;
    }
}

impl SecurityLabels {
    /// The tool's output is untrusted third-party content (a taint source).
    #[must_use]
    pub const fn is_untrusted_output(self) -> bool {
        self.contains(Self::UNTRUSTED_OUTPUT)
    }

    /// The tool sends data outbound to a third party (an egress sink).
    #[must_use]
    pub const fn is_external_send(self) -> bool {
        self.contains(Self::EXTERNAL_SEND)
    }

    /// The tool performs an irreversible / destructive effect.
    #[must_use]
    pub const fn is_irreversible(self) -> bool {
        self.contains(Self::IRREVERSIBLE)
    }

    /// Human-readable label list for structured logging (never secrets/PII).
    #[must_use]
    pub fn describe(self) -> &'static str {
        match (
            self.is_untrusted_output(),
            self.is_external_send(),
            self.is_irreversible(),
        ) {
            (false, false, false) => "none",
            (true, false, false) => "untrusted_output",
            (false, true, false) => "external_send",
            (false, false, true) => "irreversible",
            (true, true, false) => "untrusted_output|external_send",
            (true, false, true) => "untrusted_output|irreversible",
            (false, true, true) => "external_send|irreversible",
            (true, true, true) => "untrusted_output|external_send|irreversible",
        }
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
}

/// A wrapping decorator that adds no egress surface of its own delegates its
/// classification to the tool it wraps.
///
/// Used by [`crate::decorators::AuditedTool`]; kept as a free function so the
/// blanket-free delegation is written once.
#[must_use]
pub fn delegated_security_class(inner: &Arc<dyn RuntimeTool>) -> SecurityLabels {
    inner.security_class()
}

/// Implement [`RuntimeTool`] for a tool type in one line, co-located with its
/// `McpTool` impl (and therefore under the same `#[cfg(feature = ...)]`).
///
/// ```ignore
/// declare_security!(GetActivitiesTool => UNTRUSTED_OUTPUT);
/// declare_security!(DeleteCoachTool   => IRREVERSIBLE);
/// declare_security!(SetGoalTool       => empty);
/// ```
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
    ($t:ty => $($flag:ident)|+ $(,)?) => {
        impl $crate::security::RuntimeTool for $t {
            fn security_class(&self) -> $crate::security::SecurityLabels {
                $( $crate::security::SecurityLabels::$flag )|+
            }
        }
    };
}
