// ABOUTME: LLM context window management via tiered member summarization
// ABOUTME: Compresses N athletes' fitness data into token-efficient summary cards
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Write;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::models::groups::{
    GroupSummaryBlock, MemberFitnessSnapshot, MemberFlag, MemberSummaryCard, OvertrainingRiskLevel,
    SummaryDetailLevel,
};

/// Number of seconds in one hour, used by the weekly-duration formatter.
const SECONDS_PER_HOUR: f64 = 3_600.0;

/// Format the per-provider freshness map as a stable comma-separated string
/// (e.g. `"strava 33d, whoop 0d"`). Providers appear sorted by name so the
/// rendering is deterministic across snapshot fetches — the LLM is allergic
/// to nondeterministic context lines and tends to over-interpret reorderings
/// as semantic signals. Returns an empty string when the map is empty so the
/// caller can skip the line entirely instead of emitting `Sources: `.
fn format_provider_freshness(
    last_activity_per_provider: &HashMap<String, DateTime<Utc>>,
    now: DateTime<Utc>,
) -> String {
    if last_activity_per_provider.is_empty() {
        return String::new();
    }
    let mut entries: Vec<(&String, &DateTime<Utc>)> = last_activity_per_provider.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::with_capacity(entries.len() * 16);
    for (idx, (provider, latest)) in entries.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        let days = (now - **latest).num_days().max(0);
        let _ = write!(out, "{provider} {days}d");
    }
    out
}

/// Format weekly active duration as a fixed-precision hour string for prompt rendering.
///
/// Returns one decimal place (e.g. `0.0`, `1.5`, `12.3`) so the LLM sees
/// stable shape even when the value is zero, and so very short weeks
/// (~6 minutes) still render as `0.1h` rather than disappearing into `0`.
fn format_weekly_duration_hours(weekly_duration_seconds: i64) -> String {
    #[allow(clippy::cast_precision_loss)]
    // weekly_duration_seconds is capped at one week of activity time; far below f64 precision limits
    let hours = weekly_duration_seconds.max(0) as f64 / SECONDS_PER_HOUR;
    format!("{hours:.1}")
}

/// Strategy for compressing member fitness data into LLM-consumable summaries.
///
/// Implementations vary in detail level and token cost per member.
/// The `AdaptiveSummarizer` auto-selects the best level based on group size.
#[async_trait]
pub trait GroupSummarizationStrategy: Send + Sync {
    /// Summarize a single member's fitness state into a token-efficient card
    fn summarize_member(&self, snapshot: &MemberFitnessSnapshot) -> MemberSummaryCard;

    /// Combine member cards into a group summary block for prompt injection
    fn summarize_group(&self, members: &[MemberSummaryCard]) -> GroupSummaryBlock;

    /// Estimated token count for a group of the given size
    fn estimated_tokens(&self, member_count: usize) -> usize;

    /// The detail level this strategy produces
    fn detail_level(&self) -> SummaryDetailLevel;
}

/// Minimal one-line roster cards (~50 tokens per member).
/// Best for large groups (15+ members).
pub struct RosterCardSummarizer;

impl RosterCardSummarizer {
    const TOKENS_PER_MEMBER: usize = 50;

    fn build_flags(snapshot: &MemberFitnessSnapshot) -> Vec<MemberFlag> {
        let mut flags = Vec::new();

        if matches!(snapshot.overtraining_risk, OvertrainingRiskLevel::High) {
            flags.push(MemberFlag::Overreaching);
        }

        if let Some(tsb) = snapshot.tsb {
            if tsb > 10.0 {
                flags.push(MemberFlag::FreshForm);
            }
        }

        if let Some(days) = snapshot.days_since_last_activity {
            if days > 7 {
                flags.push(MemberFlag::Inactive);
            }
        }

        flags
    }
}

#[async_trait]
impl GroupSummarizationStrategy for RosterCardSummarizer {
    fn summarize_member(&self, snapshot: &MemberFitnessSnapshot) -> MemberSummaryCard {
        let flags = Self::build_flags(snapshot);

        let mut text = String::with_capacity(128);
        let _ = write!(text, "{}", snapshot.display_name);

        if let Some(ctl) = snapshot.ctl {
            let _ = write!(text, ": CTL {ctl:.0}");
        }
        if let Some(tsb) = snapshot.tsb {
            let _ = write!(text, " TSB {tsb:+.0}");
        }
        // Render duration alongside distance so HR/duration-only sources
        // (WHOOP, indoor trainers) don't collapse to a misleading "0km/wk".
        let _ = write!(
            text,
            " | {}h / {:.0}km/wk",
            format_weekly_duration_hours(snapshot.weekly_duration_seconds),
            snapshot.weekly_volume_km
        );
        if let Some(sport) = &snapshot.primary_sport {
            let _ = write!(text, " {sport}");
        }

        for flag in &flags {
            match flag {
                MemberFlag::Overreaching => text.push_str(" [OVERREACHING]"),
                MemberFlag::FreshForm => text.push_str(" [FRESH]"),
                MemberFlag::Inactive => text.push_str(" [INACTIVE]"),
                MemberFlag::PersonalRecord => text.push_str(" [PR]"),
                MemberFlag::InjuryRisk => text.push_str(" [INJURY RISK]"),
                MemberFlag::VolumeDrop => text.push_str(" [VOLUME DROP]"),
            }
        }

        MemberSummaryCard {
            user_id: snapshot.user_id,
            display_name: snapshot.display_name.clone(),
            summary_text: text,
            estimated_tokens: Self::TOKENS_PER_MEMBER,
            detail_level: SummaryDetailLevel::Roster,
            flags,
        }
    }

    fn summarize_group(&self, members: &[MemberSummaryCard]) -> GroupSummaryBlock {
        let mut text = String::with_capacity(members.len() * 80 + 64);
        text.push_str("Group Roster:\n");
        for card in members {
            let _ = writeln!(text, "- {}", card.summary_text);
        }

        GroupSummaryBlock {
            estimated_tokens: members.len() * Self::TOKENS_PER_MEMBER + 10,
            member_count: members.len(),
            text,
        }
    }

    fn estimated_tokens(&self, member_count: usize) -> usize {
        member_count * Self::TOKENS_PER_MEMBER + 10
    }

    fn detail_level(&self) -> SummaryDetailLevel {
        SummaryDetailLevel::Roster
    }
}

/// Weekly digest with key workouts and trends (~200 tokens per member).
/// Best for medium groups (5-15 members).
pub struct WeeklyDigestSummarizer;

#[async_trait]
impl GroupSummarizationStrategy for WeeklyDigestSummarizer {
    fn summarize_member(&self, snapshot: &MemberFitnessSnapshot) -> MemberSummaryCard {
        let flags = RosterCardSummarizer::build_flags(snapshot);

        let mut text = String::with_capacity(256);
        let _ = writeln!(text, "{} (this week):", snapshot.display_name);
        // Always emit duration alongside distance: HR/duration-only sources
        // (WHOOP, indoor trainers) carry no GPS and would otherwise look
        // like the athlete trained 0 km despite hours of work.
        let _ = writeln!(
            text,
            "  {} activities, {}h active, {:.1}km total",
            snapshot.weekly_activity_count,
            format_weekly_duration_hours(snapshot.weekly_duration_seconds),
            snapshot.weekly_volume_km
        );
        // Per-provider freshness so the LLM can see "strava 33d, whoop 0d"
        // and avoid attributing low volume to a "not synced" excuse.
        let freshness =
            format_provider_freshness(&snapshot.last_activity_per_provider, snapshot.computed_at);
        if !freshness.is_empty() {
            let _ = writeln!(text, "  Sources: {freshness}");
        }
        if let (Some(ctl), Some(atl), Some(tsb)) = (snapshot.ctl, snapshot.atl, snapshot.tsb) {
            let _ = writeln!(text, "  CTL: {ctl:.0} | ATL: {atl:.0} | TSB: {tsb:+.0}");
        }
        if let Some(vdot) = snapshot.vdot {
            let _ = writeln!(text, "  VDOT: {vdot:.1}");
        }
        if !flags.is_empty() {
            let flag_strs: Vec<&str> = flags
                .iter()
                .map(|f| match f {
                    MemberFlag::Overreaching => "overreaching",
                    MemberFlag::FreshForm => "fresh form",
                    MemberFlag::Inactive => "inactive",
                    MemberFlag::PersonalRecord => "new PR",
                    MemberFlag::InjuryRisk => "injury risk",
                    MemberFlag::VolumeDrop => "volume drop",
                })
                .collect();
            let _ = writeln!(text, "  Flags: {}", flag_strs.join(", "));
        }

        MemberSummaryCard {
            user_id: snapshot.user_id,
            display_name: snapshot.display_name.clone(),
            summary_text: text,
            estimated_tokens: 200,
            detail_level: SummaryDetailLevel::Weekly,
            flags,
        }
    }

    fn summarize_group(&self, members: &[MemberSummaryCard]) -> GroupSummaryBlock {
        let mut text = String::with_capacity(members.len() * 300 + 64);
        text.push_str("Group Weekly Summary:\n\n");
        for card in members {
            text.push_str(&card.summary_text);
            text.push('\n');
        }

        GroupSummaryBlock {
            estimated_tokens: members.len() * 200 + 20,
            member_count: members.len(),
            text,
        }
    }

    fn estimated_tokens(&self, member_count: usize) -> usize {
        member_count * 200 + 20
    }

    fn detail_level(&self) -> SummaryDetailLevel {
        SummaryDetailLevel::Weekly
    }
}

/// Auto-selects detail level based on group size and token budget.
/// Default strategy for most use cases.
pub struct AdaptiveSummarizer {
    /// Maximum total tokens to allocate for group context
    token_budget: usize,
}

impl AdaptiveSummarizer {
    /// Default token budget (2000 tokens for group context)
    const DEFAULT_BUDGET: usize = 2000;

    /// Create with default token budget
    #[must_use]
    pub fn new() -> Self {
        Self {
            token_budget: Self::DEFAULT_BUDGET,
        }
    }

    /// Create with custom token budget
    #[must_use]
    pub fn with_budget(token_budget: usize) -> Self {
        Self { token_budget }
    }

    fn select_strategy(&self, member_count: usize) -> Box<dyn GroupSummarizationStrategy> {
        let roster_cost = member_count * 50 + 10;
        let weekly_cost = member_count * 200 + 20;

        if weekly_cost <= self.token_budget {
            Box::new(WeeklyDigestSummarizer)
        } else if roster_cost <= self.token_budget {
            Box::new(RosterCardSummarizer)
        } else {
            // Even roster cards exceed budget — use roster anyway, it's the minimum
            Box::new(RosterCardSummarizer)
        }
    }
}

impl Default for AdaptiveSummarizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GroupSummarizationStrategy for AdaptiveSummarizer {
    fn summarize_member(&self, snapshot: &MemberFitnessSnapshot) -> MemberSummaryCard {
        // For individual members, default to weekly detail
        WeeklyDigestSummarizer.summarize_member(snapshot)
    }

    fn summarize_group(&self, members: &[MemberSummaryCard]) -> GroupSummaryBlock {
        // Re-summarize at the appropriate level based on count
        let strategy = self.select_strategy(members.len());

        // If the cards were generated at weekly level but we need roster level,
        // the text is already in the cards — just use them as-is
        strategy.summarize_group(members)
    }

    fn estimated_tokens(&self, member_count: usize) -> usize {
        let strategy = self.select_strategy(member_count);
        strategy.estimated_tokens(member_count)
    }

    fn detail_level(&self) -> SummaryDetailLevel {
        // Adaptive doesn't have a fixed level
        SummaryDetailLevel::Weekly
    }
}
