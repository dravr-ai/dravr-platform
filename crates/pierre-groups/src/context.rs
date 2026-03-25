// ABOUTME: System prompt context builders for LLM injection
// ABOUTME: Helper functions for formatting group data into prompt-ready text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use pierre_core::models::groups::GroupAggregateStats;

/// Format aggregate stats as a concise text block for prompt injection
#[must_use]
pub fn format_aggregate_stats(stats: &GroupAggregateStats) -> String {
    let mut text = String::with_capacity(256);
    let _ = writeln!(
        text,
        "Group: {}/{} members active",
        stats.active_members, stats.total_members
    );
    let _ = writeln!(text, "Avg volume: {:.1}km/wk", stats.avg_weekly_volume_km);
    if let Some(ctl) = stats.avg_ctl {
        let _ = writeln!(text, "Avg CTL: {ctl:.0}");
    }
    if stats.flagged_members > 0 {
        let _ = writeln!(
            text,
            "{} member(s) flagged for attention",
            stats.flagged_members
        );
    }
    text
}
