// ABOUTME: Shapes a get_activities list for the athlete — its rows only, capped where it folds into prose
// ABOUTME: The agent-facing header, [Today] anchor and [Note] lines never reach an athlete surface
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Shaping the activity list for the athlete.
//!
//! `get_activities` returns a pre-formatted prose block written for the agent:
//! an English panel header, a `[Today]` date anchor and `[Note]` merge lines
//! above the numbered rows. Only the rows are the athlete's, so every surface
//! gets [`athlete_list`] — a surface with
//! [`crate::BlockSupport::activity_list_card`] draws it as its own "Your
//! Activities" panel, and a surface without one has it folded into the reply
//! prose through [`shape_for_fold`].
//!
//! The fold adds one more change, a consequence of losing the panel: a raw
//! 186-row history folded into a chat bubble is unreadable, so a long list
//! collapses to its top entries plus a localized "…and N more" footer. The
//! panel scrolls, so it keeps every row.

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_BACKFILL_LIST_MORE};

/// At or below this many activities the full list is folded in; above it, the
/// list collapses to the top entries plus the localized footer.
const FULL_LIST_THRESHOLD: usize = 20;

/// Number of entries kept when a long list is collapsed.
const LIST_TOP_N: usize = 12;

/// The rows of a `get_activities` list, for any athlete surface.
///
/// Every other line the list carries is addressed to the agent, in English,
/// whatever the athlete speaks. On 2026-10-30 a French Telegram athlete was
/// handed `[Today] 2026-10-30 ven — newest activity listed: …` above their own
/// rows, because the fold dropped only the panel header. Keeping the rows
/// rather than dropping known directives keeps a directive added later on the
/// agent's side too.
///
/// Returns `None` when there is no row to show, so the caller does not have to
/// distinguish "no list" from an empty one or one holding only agent notes.
#[must_use]
pub fn athlete_list(list: Option<&str>) -> Option<String> {
    let rows: Vec<&str> = list?.lines().filter(|l| is_activity_row(l)).collect();
    (!rows.is_empty()).then(|| rows.join("\n"))
}

/// Shape a `get_activities` list for folding into reply prose.
///
/// Returns `None` when there is nothing worth folding, as [`athlete_list`].
#[must_use]
pub fn shape_for_fold(
    list: Option<&str>,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Option<String> {
    let rows = athlete_list(list)?;
    Some(cap_for_small_screen(&rows, strings, locale))
}

/// Cap a long list to its first [`LIST_TOP_N`] rows.
///
/// The rows are already in the order the request asked for, so the kept ones
/// are the ones the athlete asked to see first.
fn cap_for_small_screen(rows: &str, strings: &MessagingStringsRegistry, locale: &str) -> String {
    let total_entries = rows.lines().count();
    if total_entries <= FULL_LIST_THRESHOLD {
        return rows.to_owned();
    }

    let mut result = rows.lines().take(LIST_TOP_N).collect::<Vec<_>>().join("\n");
    let remaining = total_entries.saturating_sub(LIST_TOP_N).to_string();
    result.push('\n');
    result.push_str(&strings.render(KEY_BACKFILL_LIST_MORE, locale, &[&remaining]));
    result
}

/// Whether a line is a numbered activity row (`"<digits>. [<sport>] …"`), as
/// `format_activities_as_list` writes one. Its header, `[Today]` and `[Note]`
/// lines are not.
fn is_activity_row(line: &str) -> bool {
    line.split_once(". [")
        .is_some_and(|(prefix, _)| !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list as `get_activities` rendered it for the 2026-10-30 Telegram turn,
    /// with the merge note a split session adds.
    const AGENT_LIST: &str = "Your Activities:\n\
[Today] 2026-10-30 ven — newest activity listed: 2026-10-29 jeu (hier)\n\
\n\
[Note] 3 recordings, representing 2 distinct training sessions: auto-splits, re-uploads, a second device or another provider's copy of the same workout are merged.\n\
[Note] Each session listed already combines its recordings (count the sessions listed, never the recordings):\n\
       - session a1; recordings: [a1, a2] from strava + garmin (VTT, 2026-10-29 12:51 → 2026-10-29 14:13)\n\
\n\
1. [VTT] Montée - 2026-10-29 jeu 12:51 (hier) - 15.29 km - 1:22:15 - +337m - 2°C\n\
2. [trail] Trotinette - 2026-10-28 mer 12:56 - 6.23 km - 0:49:06 - +192m - -2°C";

    #[test]
    fn the_athlete_reads_only_the_rows_never_the_agents_english_lines() {
        let rows = athlete_list(Some(AGENT_LIST)).expect("two rows");
        for agent_line in [
            "Your Activities",
            "[Today]",
            "newest activity listed",
            "[Note]",
            "session a1",
        ] {
            assert!(!rows.contains(agent_line), "{agent_line} leaked: {rows}");
        }
        assert_eq!(rows.lines().count(), 2, "{rows}");
        assert!(rows.starts_with("1. [VTT] Montée"), "{rows}");
    }

    #[test]
    fn a_list_with_no_row_is_no_list() {
        assert_eq!(
            athlete_list(Some("Your Activities:\n[Today] 2026-10-30 ven\n")),
            None
        );
        assert_eq!(athlete_list(None), None);
    }
}
