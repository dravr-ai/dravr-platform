// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The activity-sport vocabulary the clients share, read by the server so a messaging reply names a sport in the athlete's locale
// ABOUTME: One JSON table in packages/shared-constants feeds both sides; the fold and alias rules mirror activitySportLabelKey exactly

//! The activity-sport vocabulary, shared with the clients.
//!
//! Provider activity sports (`Run`, `TrailRun`, `VirtualRide`, ...) arrive as
//! wire spellings. The clients fold them onto a canonical name and resolve a
//! catalogue key (`app.sportRun`) through `activitySportLabelKey`; this module
//! is the same table and the same fold for server-rendered text, so the
//! onboarding coach proposal on Telegram names the sport in French exactly as
//! the web step does.

use std::collections::HashMap;
use std::sync::LazyLock;

use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use serde::Deserialize;

/// The shared table: canonical sport → catalogue key, plus wire aliases.
#[derive(Deserialize)]
struct ActivitySports {
    #[serde(rename = "labelKeys")]
    label_keys: HashMap<String, String>,
    aliases: HashMap<String, String>,
}

/// Embedded at build time from the same file the clients import, so the two
/// vocabularies cannot drift.
///
/// Kept as the parse result rather than unwrapped: a malformed file makes
/// every lookup answer `None` (the wire text is kept) and fails the test that
/// pins `Run` to `app.sportRun`, instead of panicking at first use.
static TABLE: LazyLock<Result<ActivitySports, serde_json::Error>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../packages/shared-constants/src/activity-sports.json"
    ))
});

/// The catalogue key naming `sport` for an athlete.
///
/// `None` for a spelling the vocabulary has no word for — the caller keeps
/// the wire text then. Folds the way the client does: trim, lowercase, spaces and hyphens to
/// underscores, a trailing `_v<n>` version suffix dropped, then the alias map.
#[must_use]
pub fn activity_sport_label_key(sport: &str) -> Option<&'static str> {
    let folded = sport.trim().to_lowercase();
    let mut canonical = String::with_capacity(folded.len());
    let mut last_sep = false;
    for ch in folded.chars() {
        if ch.is_whitespace() || ch == '-' || ch == '_' {
            if !last_sep {
                canonical.push('_');
            }
            last_sep = true;
        } else {
            canonical.push(ch);
            last_sep = false;
        }
    }
    let table = TABLE.as_ref().ok()?;
    let canonical = strip_version_suffix(&canonical);
    let canonical = table
        .aliases
        .get(canonical)
        .map_or(canonical, String::as_str);
    table.label_keys.get(canonical).map(String::as_str)
}

/// `virtual_ride_v2` → `virtual_ride`; anything without the suffix is returned as is.
fn strip_version_suffix(name: &str) -> &str {
    if let Some((head, tail)) = name.rsplit_once("_v") {
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            return head;
        }
    }
    name
}

/// The athlete's word for `sport` in `locale`, or the wire spelling when the
/// vocabulary has no word for it.
///
/// The one seam server-rendered text goes through, so a reply naming a sport
/// reads like the web step naming the same sport. An empty catalogue row is
/// treated as no word rather than as an empty sentence fragment.
#[must_use]
pub fn sport_label(registry: &MessagingStringsRegistry, sport: &str, locale: &str) -> String {
    activity_sport_label_key(sport)
        .map(|key| registry.get(key, locale))
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| sport.to_owned())
}
