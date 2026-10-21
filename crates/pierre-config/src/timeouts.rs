// ABOUTME: Operator-tunable time bounds on request paths, read from the environment once at startup
// ABOUTME: MCP task fast path, messaging turn watchdog, Home route reads and background revalidation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};
use std::env;

/// Time bounds a deployment may retune without a new binary, as the
/// environment set them at startup.
///
/// Each field is `None` when its variable is unset or does not hold a usable
/// value; the code that applies the bound then uses its own compiled-in
/// default, so a typo never disarms a bound.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ServerTimeouts {
    /// `PIERRE_MCP_TASK_FAST_PATH_MS`: how long a task-capable MCP call runs
    /// inline before it becomes a task handle. `0` converts every such call.
    pub mcp_task_fast_path_ms: Option<u64>,
    /// `MESSAGING_TURN_WATCHDOG_SECS`: the wall-clock ceiling of one messaging
    /// turn (positive).
    pub turn_watchdog_secs: Option<u64>,
    /// `PIERRE_HOME_ROUTE_ANSWER_SECS`: how long a Home route request waits
    /// for its read before answering pending (positive).
    pub home_route_answer_secs: Option<u64>,
    /// `PIERRE_HOME_ROUTE_PROVIDER_READ_SECS`: the bound of one provider read
    /// for a route (positive).
    pub home_route_provider_read_secs: Option<u64>,
    /// `PIERRE_REVALIDATION_TIMEOUT_SECS`: the bound a background revalidation
    /// runs under (positive).
    pub revalidation_timeout_secs: Option<u64>,
}

impl ServerTimeouts {
    /// Read the bounds from the environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            mcp_task_fast_path_ms: number("PIERRE_MCP_TASK_FAST_PATH_MS"),
            turn_watchdog_secs: positive("MESSAGING_TURN_WATCHDOG_SECS"),
            home_route_answer_secs: positive("PIERRE_HOME_ROUTE_ANSWER_SECS"),
            home_route_provider_read_secs: positive("PIERRE_HOME_ROUTE_PROVIDER_READ_SECS"),
            revalidation_timeout_secs: positive("PIERRE_REVALIDATION_TIMEOUT_SECS"),
        }
    }
}

/// The number `key` holds, `None` when unset or not a number.
fn number(key: &str) -> Option<u64> {
    env::var(key).ok().and_then(|raw| raw.trim().parse().ok())
}

/// The positive number `key` holds, `None` when unset, not a number or zero.
fn positive(key: &str) -> Option<u64> {
    number(key).filter(|value| *value > 0)
}
