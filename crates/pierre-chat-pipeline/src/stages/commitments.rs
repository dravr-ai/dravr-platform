// ABOUTME: Prompt-assembly stage that renders the athlete's open commitments into the coach's system prompt
// ABOUTME: State only — the discipline for recording a new one lives in the commitment_create tool description
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Open-commitment prompt block.
//!
//! Renders the promises the athlete has made and not yet been held to, so the
//! coach can reference them, and so it has the ids it needs to retract one.
//!
//! The block carries *state* and nothing else. The rule for when a new
//! commitment may be recorded — the athlete named a count and a deadline in
//! their own words, not a bare "ok" to the coach's suggestion — lives in
//! `commitment_create`'s tool description, which is already in the prompt for
//! every turn the tool is offered. Repeating it here would spend tokens on
//! every turn to say something the model has already been told, and prompt
//! budget in this harness is not free.
//!
//! Dates render in the athlete's own calendar. A commitment due "Sunday" that
//! the prompt calls Monday is worse than no block at all.

use std::fmt::Write as _;

use chrono::{DateTime, Duration, Utc};
use pierre_memory::commitments::Commitment;
use pierre_runtime_context::DataContext;

/// How many open commitments to render. Beyond a handful the athlete has not
/// made promises, they have made a training plan, and the block would crowd out
/// the rest of the prompt.
const MAX_COMMITMENTS_RENDERED: usize = 5;

/// Ceiling on the rows fetched before truncation.
const COMMITMENT_FETCH_LIMIT: i64 = 20;

/// Longest statement echoed into the prompt.
///
/// The stored value is already bounded at write time; this is the second line
/// of defense, alongside the whitespace collapse below, so a stored statement
/// can never open what looks like a new prompt section.
const MAX_RENDERED_STATEMENT: usize = 120;

/// Collapse a stored statement into a single safe prompt line.
///
/// Newlines and control characters go first — a statement containing `\n##` or
/// a fake instruction line is the only way this field could act on the coach —
/// then the result is truncated on a character boundary.
fn fence_statement(raw: &str) -> String {
    let flattened: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = flattened.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.char_indices().nth(MAX_RENDERED_STATEMENT) {
        Some((idx, _)) => format!("{}…", &collapsed[..idx]),
        None => collapsed,
    }
}

/// Render one commitment as a prompt bullet.
fn render_line(commitment: &Commitment, timezone: Option<&str>) -> String {
    let target = commitment.target_sessions;
    let what = commitment.sport.as_deref().map_or_else(
        || format!("{target} sessions"),
        |sport| format!("{target} × {sport}"),
    );
    let due = local_date(commitment.window_end, timezone);
    format!(
        "- {what} by {due} — \"{}\" [id: {}]",
        fence_statement(&commitment.statement),
        commitment.id
    )
}

/// Format an instant as the athlete's civil date.
///
/// `window_end` is local midnight *after* the due day, so a second is
/// subtracted to land back on the day the athlete was actually given.
fn local_date(window_end: DateTime<Utc>, timezone: Option<&str>) -> String {
    let tz: chrono_tz::Tz = timezone
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    (window_end - Duration::seconds(1))
        .with_timezone(&tz)
        .format("%a %-d %b")
        .to_string()
}

/// Build the block for a set of commitments, or `None` when there are none.
///
/// Split from the repository read so it can be asserted on directly.
#[must_use]
pub fn render_commitments_block(
    commitments: &[Commitment],
    timezone: Option<&str>,
) -> Option<String> {
    if commitments.is_empty() {
        return None;
    }
    let mut block = String::from("\n\n## Commitments the athlete made\n\n");
    for commitment in commitments.iter().take(MAX_COMMITMENTS_RENDERED) {
        let _ = writeln!(block, "{}", render_line(commitment, timezone));
    }
    block.push_str(
        "\nThese are their words, not your prescriptions. Each is counted against their real \
         activity data when its window closes, and they hear the result — so do not pre-empt it \
         with a verdict of your own. If they are dropping one, call `commitment_cancel` with its id.",
    );
    Some(block)
}

/// Append the athlete's open commitments to the system prompt.
///
/// Scoped to the TOOL tenant — the one the commitment was written under and the
/// one the athlete's activity data lives in. Best-effort: a repository failure
/// logs and leaves the prompt untouched rather than failing the turn.
pub async fn inject_commitments(
    data: &DataContext,
    tool_tenant_id: &str,
    user_id: &str,
    timezone: Option<&str>,
    base_prompt: String,
) -> String {
    let commitments = match data
        .repos()
        .commitments
        .list_open_commitments(tool_tenant_id, user_id, COMMITMENT_FETCH_LIMIT)
        .await
    {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list open commitments");
            return base_prompt;
        }
    };

    match render_commitments_block(&commitments, timezone) {
        Some(block) => format!("{base_prompt}{block}"),
        None => base_prompt,
    }
}
