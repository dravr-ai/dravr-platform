// ABOUTME: Shared vocabulary for the data tools — coverage notes, cache eligibility, annotations
// ABOUTME: Split out of data.rs, which was frozen over the size ceiling and could not grow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Helpers shared by the data-access tools.
//!
//! These were defined inside `data.rs` next to `GetActivitiesTool`, but five of
//! them are read from other files — `read_only_annotations` from twelve — so
//! they were already a shared vocabulary living in one tool's implementation.
//! Splitting them out is what the file-size gate asks for when a frozen file
//! needs to change, and it puts each item where its callers can see it.

use std::env;

use serde_json::{json, Value};

use pierre_formatters::OutputFormat;
use pierre_mcp_schema::ToolAnnotations;

/// When a historical `get_activities` query leaves `before` open, the backfill
/// gate checks cache coverage over `[after, after + 1 year]` so recent rows in
/// `[after, now]` don't mask a missing historical season.
pub const HISTORICAL_COVERAGE_BOUND_SECS: i64 = 365 * 24 * 60 * 60;

pub use crate::activity_fetch::HISTORICAL_WINDOW_READ_LIMIT;

/// Read limit (as `usize`) for the historical backfill window read.
#[must_use]
pub const fn historical_window_read_limit() -> usize {
    HISTORICAL_WINDOW_READ_LIMIT
}

/// Default fetch limit a background backfill passes to the provider so the scrape
/// pages the WHOLE requested window instead of stopping at the user's display
/// limit. The sciotte date-bounded scrape stops on `in_window_count >= limit`
/// OR `oldest <= after`; with a small (display) limit it caps at the recent tail
/// and never reaches the window start. A generous limit makes `oldest <= after`
/// the binding condition, so the scrape pages the full season.
const DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT: usize = 2_000;

/// Fetch limit a background backfill requests, from
/// `PIERRE_HISTORICAL_BACKFILL_FETCH_LIMIT` (falls back to
/// [`DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT`]). Operators raise this for athletes
/// with very deep histories; zero/unparseable values fall back to the default.
pub fn historical_backfill_fetch_limit() -> usize {
    env::var("PIERRE_HISTORICAL_BACKFILL_FETCH_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT)
}

/// Whether `provider`'s connection is in an unusable state (`NeedsReauth` /
/// `Revoked`) and so needs an interactive reconnect.
///
/// The historical gate consults this BEFORE spawning a background backfill: a
/// dead session would just fail the scrape and leave the user looping on
/// "fetching, ask again shortly". When true, `get_activities` returns
/// `provider_auth_required` instead, so the chat pipeline hands back the
/// reconnect link this turn.
///
/// Re-exported from [`pierre_core::models::connection_needs_reauth`] — the
/// single home for the scan, shared with `AuthService`. Kept re-exported here
/// so the reconnect-gate integration test can exercise the decision via this
/// module.
pub use pierre_core::models::connection_needs_reauth;

/// Whether a `get_activities` query may use the `CacheKey` response cache.
///
/// One decision governs BOTH the read short-circuit and the write-through, so
/// the two never diverge. Three query shapes must bypass the response cache:
///
/// * `auto_promote_to_detail` — the cache key omits `mode`, so a cached summary
///   cannot satisfy a detail-promoted response, and a detail payload must not be
///   written under a key a summary request would later read.
/// * `is_historical` — a deep historical window must route through the
///   coverage-aware gate, never a TTL'd response that could keep serving a stale
///   slice after the coverage was purged. Excluding it from the WRITE too keeps
///   the cache from accumulating historical entries that are never read back.
/// * `is_custom_sort` — the cache key omits `sort_by`, and the cached-serve path
///   re-orders by date, so a cached entry cannot satisfy a "longest/oldest/…"
///   ask. A non-default sort therefore bypasses the cache on both read and write.
///
/// `pub` so the read/write contract is exercisable by the integration test suite.
#[must_use]
pub fn response_cache_eligible(
    auto_promote_to_detail: bool,
    is_historical: bool,
    is_custom_sort: bool,
) -> bool {
    !auto_promote_to_detail && !is_historical && !is_custom_sort
}

/// Build the LLM-facing `coverage` sidecar for a served activity window.
///
/// Returns `Some` only when the requested window held more activities than the
/// returned slice (`window_total > returned`) — i.e. the display limit hid older
/// rows. The note steers the model to frame its reply around the full count +
/// span instead of the oldest shown activity. `None` (window fully returned, or
/// `window_total` unknown) keeps the response clean.
///
/// Lives in the tool RESULT, not the tool's `input_schema`/description, so it
/// adds no SDK or contremaitre drift; the model renders it in the user's
/// language, so it needs no localized `messaging_strings` key.
#[must_use]
pub fn activity_coverage_note(
    window_total: Option<usize>,
    returned: usize,
    window_span: Option<&(String, String)>,
) -> Option<Value> {
    let total = window_total?;
    if total <= returned {
        return None;
    }
    // When the served window filled the durable read cap the true count is
    // unknown — `window_total` is only a lower bound, so state it as "at least
    // {total}" instead of an exact figure. The live-fetch path is bounded by the
    // display limit (<= MAX_ACTIVITY_LIMIT) and never reaches the cap, so this
    // only marks a genuinely cap-truncated historical read.
    let total_phrase = if total >= HISTORICAL_WINDOW_READ_LIMIT {
        format!("at least {total}")
    } else {
        total.to_string()
    };
    window_span.map_or_else(
        || {
            Some(json!({
                "window_total": total,
                "returned": returned,
                "note": format!(
                    "This window holds {total_phrase} activities; only {returned} of them are shown below. Tell the user the full count ({total_phrase}), and don't assume the shown activities are the only ones."
                ),
            }))
        },
        |(oldest, newest)| {
            Some(json!({
                "window_total": total,
                "returned": returned,
                "window_oldest": oldest,
                "window_newest": newest,
                "note": format!(
                    "This window holds {total_phrase} activities spanning {oldest} to {newest}; only {returned} are shown below. Frame your reply around the full count ({total_phrase}) and span — do not imply the user's history is limited to the activities shown."
                ),
            }))
        },
    )
}

/// Build the `reconnect_required` sidecar for a window that was served WITHOUT
/// a connection the athlete has to re-authorize.
///
/// A multi-source aggregator serves what it holds and prompts for the dead
/// source alongside: an athlete with years of Strava behind a healthy connection
/// is owed that answer even while their watch token is expired. This sidecar is
/// how the reconnect signal survives the merge instead of being the only thing
/// the turn manages to say.
///
/// Lives in the tool RESULT, not the tool's `input_schema`/description, so it
/// adds no SDK or contremaitre drift. `pub` so the caveat's shape is
/// exercisable by the integration test suite.
///
/// Two readers, and they need different halves. `provider` and `note` address
/// the model, in the vocabulary an athlete uses: `reconnect_required` is on
/// `tool_results::ACTIVITIES_ENVELOPE_KEPT`, so the whole sidecar survives the
/// projection every prompt-facing render runs the payload through, and the
/// coach reads that the window it is about to answer from is missing a source.
/// `provider_slug` is the backend key `tool_results::reconnect_offer_in_responses`
/// lifts out for the chat pipeline to mint a reconnect URL from —
/// `sciotte_garmin` takes the Dravr-hosted login page and `garmin` takes an
/// OAuth authorization round-trip, so the display name cannot stand in for it.
#[must_use]
pub fn provider_reconnect_note(display_name: &str, backend: &str) -> Value {
    json!({
        "provider": display_name,
        "provider_slug": backend,
        "note": format!(
            "The activities above were served WITHOUT {display_name}: that connection expired and the athlete must re-authorize it. Answer the question from the activities shown, then add one short sentence that {display_name} is disconnected and that reconnecting it restores the sessions only it records."
        ),
    })
}

/// Pull the `format` arg, defaulting to JSON.
pub fn parse_output_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map_or(OutputFormat::Json, OutputFormat::from_str_param)
}

/// Annotations for read-only data retrieval tools
pub fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// GetActivitiesTool - Retrieve user activities
// ============================================================================
