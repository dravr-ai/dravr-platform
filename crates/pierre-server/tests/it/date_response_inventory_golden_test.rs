// ABOUTME: The tools that have a response date golden, checked against the names the server registers
// ABOUTME: Prints how many tools are registered and how many are covered, so the coverage figure is a command's output
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: which tools have one (carnet#1620, gate 3).
//!
//! The coverage of gate 3 is stated against the tools the server registers,
//! so the denominator is the registry's own count and not a grep. Run with
//! `--nocapture` to read both numbers:
//!
//! ```text
//! cargo test -p pierre_mcp_server --test it -- date_response_inventory --nocapture
//! ```
//!
//! The covered tools are read, not counted by hand: they are the registered
//! names that a `date_response_*` suite writes as a string literal, read
//! from the suites' own sources (embedded when this binary is compiled, so
//! the count needs no checkout). A golden deleted with its suite stops
//! counting; a new suite missing from [`SUITES`] fails, because the list is
//! checked against the `mod date_response_*` lines of `main.rs`. The set
//! read is then compared with the literal [`COVERED`], so a tool that loses
//! its golden, or gains one, shows as a diff here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeSet;

use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;
use regex::Regex;

/// Every `date_response_*` suite but this one, by module name and source.
///
/// Read as text because the suites are the data under test: which registered
/// tool names they call is the coverage, and no build exposes it otherwise.
/// No production source is read.
const SUITES: &[(&str, &str)] = &[
    (
        "date_response_a2a_golden_test",
        include_str!("date_response_a2a_golden_test.rs"),
    ),
    (
        "date_response_account_golden_test",
        include_str!("date_response_account_golden_test.rs"),
    ),
    (
        "date_response_activity_tools_golden_test",
        include_str!("date_response_activity_tools_golden_test.rs"),
    ),
    (
        "date_response_admin_console_golden_test",
        include_str!("date_response_admin_console_golden_test.rs"),
    ),
    (
        "date_response_admin_golden_test",
        include_str!("date_response_admin_golden_test.rs"),
    ),
    (
        "date_response_athlete_home_golden_test",
        include_str!("date_response_athlete_home_golden_test.rs"),
    ),
    (
        "date_response_chat_golden_test",
        include_str!("date_response_chat_golden_test.rs"),
    ),
    (
        "date_response_coach_attention_golden_test",
        include_str!("date_response_coach_attention_golden_test.rs"),
    ),
    (
        "date_response_coach_golden_test",
        include_str!("date_response_coach_golden_test.rs"),
    ),
    (
        "date_response_groups_golden_test",
        include_str!("date_response_groups_golden_test.rs"),
    ),
    (
        "date_response_health_tools_golden_test",
        include_str!("date_response_health_tools_golden_test.rs"),
    ),
    (
        "date_response_history_tools_golden_test",
        include_str!("date_response_history_tools_golden_test.rs"),
    ),
    (
        "date_response_invitations_golden_test",
        include_str!("date_response_invitations_golden_test.rs"),
    ),
    (
        "date_response_messaging_golden_test",
        include_str!("date_response_messaging_golden_test.rs"),
    ),
    (
        "date_response_segments_golden_test",
        include_str!("date_response_segments_golden_test.rs"),
    ),
    (
        "date_response_session_golden_test",
        include_str!("date_response_session_golden_test.rs"),
    ),
    (
        "date_response_today_golden_test",
        include_str!("date_response_today_golden_test.rs"),
    ),
];

/// The registered tools the suites write as a literal, in name order.
const COVERED: &[&str] = &[
    "analyze_activity",
    "compute_training_history",
    "get_activities",
    "get_activity_intelligence",
    "get_health_snapshots",
    "get_recovery_metrics",
    "get_roster_overview",
    "get_segment_history",
    "get_sleep_sessions",
    "get_training_history",
    "get_training_plan",
    "set_goal",
    "track_progress",
];

/// The test binary's module list, read to check [`SUITES`] against it: the
/// two lists are joined by nothing at runtime, so only a parity read catches
/// a suite added to one and not the other.
const MAIN: &str = include_str!("main.rs");
const THIS: &str = "date_response_inventory_golden_test";

#[test]
fn the_suites_read_here_are_every_date_response_suite() {
    let declared: BTreeSet<&str> = Regex::new(r"(?m)^mod (date_response_\w+);")
        .unwrap()
        .captures_iter(MAIN)
        .map(|found| found.get(1).unwrap().as_str())
        .filter(|suite| *suite != THIS)
        .collect();
    let read: BTreeSet<&str> = SUITES.iter().map(|(suite, _)| *suite).collect();
    assert_eq!(read, declared, "SUITES names every date_response_* module");
}

#[test]
fn the_tools_with_a_date_golden_are_read_from_the_suites() {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let registered: BTreeSet<&str> = registry.tool_names().into_iter().collect();

    let literal = Regex::new(r#""([a-z][a-z0-9_]*)""#).unwrap();
    let covered: BTreeSet<&str> = SUITES
        .iter()
        .flat_map(|(_, source)| literal.captures_iter(source))
        .map(|found| found.get(1).unwrap().as_str())
        .filter_map(|name| registered.get(name).copied())
        .collect();
    assert_eq!(
        covered.iter().copied().collect::<Vec<_>>(),
        COVERED,
        "the tools with a date golden"
    );
    println!(
        "registered tools: {}; with a date golden: {}",
        registered.len(),
        covered.len()
    );
    println!(
        "with one: {}",
        covered.iter().copied().collect::<Vec<_>>().join(" ")
    );
    let without: Vec<&str> = registered.difference(&covered).copied().collect();
    println!("without one: {}", without.join(" "));
}
