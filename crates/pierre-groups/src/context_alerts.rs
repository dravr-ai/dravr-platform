// ABOUTME: Renders the member-state alert sections appended to the group context
// ABOUTME: Connection alerts (broken) and stale-snapshot directives (unrefreshed) — quiet needs none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Alert sections for the injected group context.
//!
//! A member the coach cannot fully read is in one of three states, each with
//! its own rendering: **broken** (a provider connection died — Connection
//! alerts), **stale** (the snapshot's cache could not be refreshed this turn —
//! Stale snapshots), and **quiet** (fresh snapshot, no recent training), which
//! needs no section at all because the numbers speak for themselves.

use pierre_core::models::groups::MemberFitnessSnapshot;

/// Render the Connection-alerts and Stale-snapshots sections for the visible
/// member set, or an empty string when every member is healthy and fresh.
///
/// Gated identically to the snapshot cards: callers pass only
/// visible/consenting members, so neither section can leak a hidden member's
/// state. A member's own reconnect link is delivered out-of-band — never here.
pub fn connection_and_staleness_alerts(visible_snapshots: &[&MemberFitnessSnapshot]) -> String {
    // Connection alerts: name any visible member whose provider connection died
    // so the coach reports the dead provider instead of treating it as merely
    // quiet.
    let mut reauth_lines: Vec<String> = visible_snapshots
        .iter()
        .filter(|s| !s.needs_reauth_providers.is_empty())
        .map(|s| {
            format!(
                "- {} needs to reconnect: {}",
                s.display_name,
                s.needs_reauth_providers.join(", ")
            )
        })
        .collect();
    let reauth_alerts = if reauth_lines.is_empty() {
        String::new()
    } else {
        reauth_lines.sort();
        format!(
            "\n\n## Connection alerts\n\
            These members have a disconnected provider — you cannot pull their fresh data \
            for it. Tell them to reconnect and do not invent data for a disconnected \
            source:\n{}",
            reauth_lines.join("\n")
        )
    };

    // Stale snapshots: the third member state, distinct from both "quiet"
    // (fresh snapshot, no recent training) and "broken" (connection alerts
    // above). A stale snapshot's numbers and activity dates describe an old
    // cache, and without this directive the model has been observed narrating
    // that age as a provider fault ("son Strava n'a pas resynchronisé depuis
    // 33 jours — il devrait le reconnecter") while the member's connection was
    // healthy — the 2026-08-13 incident where that story wrapped an inverted
    // recovery verdict.
    let mut stale_lines: Vec<String> = visible_snapshots
        .iter()
        .filter(|s| s.served_stale)
        .map(|s| format!("- {}", s.display_name))
        .collect();
    let stale_directives = if stale_lines.is_empty() {
        String::new()
    } else {
        stale_lines.sort();
        format!(
            "\n\n## Stale snapshots\n\
            These members' snapshots were served from a cache that could not be \
            refreshed for this turn. Before answering ANYTHING about them, call \
            `get_group_member_activities` to fetch their current data. A stale \
            snapshot's activity dates say nothing about the member's provider \
            connection — never tell anyone to reconnect or resync a provider unless \
            it is listed under Connection alerts:\n{}",
            stale_lines.join("\n")
        )
    };

    format!("{reauth_alerts}{stale_directives}")
}
