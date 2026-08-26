// ABOUTME: Shapes a get_activities list for a surface that has no activity panel to draw it in
// ABOUTME: Drops the panel header and caps a long history so it cannot overflow a chat bubble
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Shaping the activity list for a surface without a panel.
//!
//! `get_activities` returns a pre-formatted prose block, and a surface with
//! [`crate::BlockSupport::activity_list_card`] draws it as its own "Your
//! Activities" panel above the coach's analysis. A surface without one has the
//! list folded into the reply prose instead — and a raw 186-row history folded
//! into a chat bubble is unreadable, so it is shaped first.
//!
//! Two changes, both of them consequences of losing the panel: the panel's own
//! English header is dropped (it is a card header, not a sentence, and it
//! reads as noise above the entries on an often non-English channel), and a
//! long list collapses to its top entries plus a localized "…and N more"
//! footer. Neither applies where the panel exists, because the panel scrolls.

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_BACKFILL_LIST_MORE};

/// At or below this many activities the full list is folded in; above it, the
/// list collapses to the top entries plus the localized footer.
const FULL_LIST_THRESHOLD: usize = 20;

/// Number of entries kept when a long list is collapsed.
const LIST_TOP_N: usize = 12;

/// The panel header emitted by `format_activities_as_list`. A card header on a
/// surface that has a card; dead weight on one that does not.
const PANEL_HEADER: &str = "Your Activities:";

/// Shape a `get_activities` list for folding into reply prose.
///
/// Returns `None` when there is nothing worth folding, so the caller does not
/// have to distinguish "no list" from "an empty one".
#[must_use]
pub fn shape_for_fold(
    list: Option<&str>,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Option<String> {
    let list = list?;
    if list.trim().is_empty() {
        return None;
    }
    let list = list
        .strip_prefix(PANEL_HEADER)
        .map_or(list, str::trim_start);
    Some(cap_for_small_screen(list, strings, locale))
}

/// Cap a long list, keeping the header/notes lines and the first
/// [`LIST_TOP_N`] numbered entries.
///
/// The entries are already in the order the request asked for, so the kept
/// ones are the ones the athlete asked to see first.
fn cap_for_small_screen(prose: &str, strings: &MessagingStringsRegistry, locale: &str) -> String {
    let total_entries = prose.lines().filter(|l| is_numbered_entry(l)).count();
    if total_entries <= FULL_LIST_THRESHOLD {
        return prose.to_owned();
    }

    let mut kept = 0usize;
    let mut out: Vec<&str> = Vec::with_capacity(LIST_TOP_N + 4);
    for line in prose.lines() {
        if is_numbered_entry(line) {
            if kept < LIST_TOP_N {
                out.push(line);
                kept += 1;
            }
        } else {
            out.push(line);
        }
    }

    let remaining = total_entries.saturating_sub(LIST_TOP_N).to_string();
    let footer = strings.render(KEY_BACKFILL_LIST_MORE, locale, &[&remaining]);
    let mut result = out.join("\n");
    result.push('\n');
    result.push_str(&footer);
    result
}

/// Whether a prose line is a numbered activity entry (`"<digits>. [<sport>] …"`),
/// as emitted by `format_activities_as_list`. Header and `[Note]` lines are not.
fn is_numbered_entry(line: &str) -> bool {
    line.split_once(". [")
        .is_some_and(|(prefix, _)| !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()))
}
