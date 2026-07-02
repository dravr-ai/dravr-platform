// ABOUTME: Renders a user's proven coaching playbooks into a system-prompt block
// ABOUTME: Only well-evidenced, confident playbooks are surfaced; enums + counters + capture-sanitized sport slug
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Playbook → prompt rendering (P5 of coaching playbook memory).
//!
//! Turns the most-confident learned [`Playbook`]s for an athlete into a compact
//! markdown block the coach can reason over. Every rendered field is either a
//! system-derived enum, a counter, or the `sport` slug — and `sport` is
//! constrained to a bounded `[a-z0-9_]` slug at capture
//! (`advice_capture::sanitize_sport`), not free text. So (unlike the OKF bundle)
//! no StruQ fencing is needed here.

use std::collections::HashSet;
use std::fmt::Write as _;

use pierre_memory::playbooks::{ArchetypePrior, Playbook};

/// Minimum decisive outcomes (success + failure) before a playbook is trusted
/// enough to surface — keeps lucky one-offs out of the prompt.
const MIN_EVIDENCE: u32 = 3;
/// Minimum Wilson-lower-bound confidence to surface a playbook (personal or prior).
const MIN_CONFIDENCE: f32 = 0.5;
/// Cap on playbooks rendered into a single prompt.
const MAX_PLAYBOOKS_RENDERED: usize = 5;
/// Cap on archetype (cold-start) priors rendered into a single prompt.
const MAX_ARCHETYPE_RENDERED: usize = 3;

/// Render the qualifying playbooks (already confidence-sorted by the repository)
/// into a prompt block, or `None` when none clear the evidence/confidence bar.
#[must_use]
pub fn render_playbooks_block(playbooks: &[Playbook]) -> Option<String> {
    let qualified: Vec<&Playbook> = playbooks
        .iter()
        .filter(|p| {
            p.success_count.saturating_add(p.failure_count) >= MIN_EVIDENCE
                && p.confidence >= MIN_CONFIDENCE
        })
        .take(MAX_PLAYBOOKS_RENDERED)
        .collect();
    if qualified.is_empty() {
        return None;
    }
    let mut out =
        String::from("\n\n## What works for this athlete (learned from their own outcomes)\n\n");
    for p in qualified {
        let decisive = p.success_count.saturating_add(p.failure_count);
        let sport = p
            .trigger
            .sport
            .as_deref()
            .map_or_else(String::new, |s| format!(", {s}"));
        // `write!` to a String is infallible; ignore the formatter Result.
        let _ = writeln!(
            out,
            "- When {} ({}{sport}): {} worked {}/{decisive} times ({:.0}% confidence).",
            humanize(p.trigger.kind.as_str()),
            p.trigger.magnitude.as_str(),
            humanize(p.intervention.kind.as_str()),
            p.success_count,
            f64::from(p.confidence) * 100.0,
        );
    }
    out.push_str(
        "\nThese are patterns learned from this athlete's own data — prefer what has worked and avoid what hasn't. They inform your judgment; they do not override the athlete's stated constraints or a human coach.\n",
    );
    Some(out)
}

/// Render cold-start archetype priors ("patterns from similar athletes") that
/// the given personal playbooks do **not** already cover, or `None` when none
/// qualify.
///
/// Excludes any `(trigger, intervention)` the athlete already has a personal
/// playbook for (their own data wins), and filters to confident priors. The
/// priors are already k-anonymous aggregates; this is purely presentation.
#[must_use]
pub fn render_archetype_block(priors: &[ArchetypePrior], personal: &[Playbook]) -> Option<String> {
    let covered: HashSet<(String, String)> = personal
        .iter()
        .map(|p| (p.trigger.hash_key(), p.intervention.hash_key()))
        .collect();
    let qualified: Vec<&ArchetypePrior> = priors
        .iter()
        .filter(|p| {
            p.confidence >= MIN_CONFIDENCE
                && !covered.contains(&(p.trigger.hash_key(), p.intervention.hash_key()))
        })
        .take(MAX_ARCHETYPE_RENDERED)
        .collect();
    if qualified.is_empty() {
        return None;
    }
    let mut out = String::from(
        "\n\n## Patterns from similar athletes (cold-start guidance, weak priors)\n\n",
    );
    for p in qualified {
        let decisive = p.success_count.saturating_add(p.failure_count);
        let sport = p
            .trigger
            .sport
            .as_deref()
            .map_or_else(String::new, |s| format!(", {s}"));
        let _ = writeln!(
            out,
            "- When {} ({}{sport}): {} succeeded {}/{decisive} times across {} similar athletes ({:.0}% confidence).",
            humanize(p.trigger.kind.as_str()),
            p.trigger.magnitude.as_str(),
            humanize(p.intervention.kind.as_str()),
            p.success_count,
            p.distinct_user_count,
            f64::from(p.confidence) * 100.0,
        );
    }
    out.push_str(
        "\nThese are population patterns, not this athlete's own data — treat them as weak starting priors and defer to their personal playbooks and stated preferences as those emerge.\n",
    );
    Some(out)
}

/// Turn an enum slug (`hrv_drop`) into prose-ish text (`hrv drop`) for the prompt.
fn humanize(slug: &str) -> String {
    slug.replace('_', " ")
}
