// ABOUTME: A detail walk's ask while one is owed: the statement that widens its floor and nothing else
// ABOUTME: The cursor a step moved stays, the total is recounted, two different sports read every sport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// What a newer ask widens an owed detail walk to
/// ([`super::ActivityBackfillJobRepository::widen_detail_walk`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkWiden {
    /// The new ask's window floor (unix seconds): the row's `after_ts`
    /// becomes the lower of the two, never the higher.
    pub after_ts: i64,
    /// The new ask's sport type; `None` for every sport. The row keeps its
    /// sport only when the two are the same one.
    pub walk_sport: Option<String>,
    /// Activities the widened window leaves unread, counted by the asker:
    /// the row's [`super::JobProgress::units_total`] becomes what it has done
    /// plus these.
    pub units_left: i64,
}

/// Widens the floor of the detail walk owed for the pair (`$4` its kind) and
/// nothing else, in the row's own tenant. Its cursor, `before_ts`, is left
/// where its steps moved it; a newer top is the next walk's, once this one
/// ends. A NULL floor reads no lower bound, the widest there is, and stays.
/// A sport kept only when both asks name the same one: a NULL on either side
/// reads every sport. A step's progress write keeps `after_ts`, so the two
/// never race on it.
pub(crate) const WIDEN_DETAIL_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET after_ts = CASE WHEN after_ts IS NOT NULL AND $5 < after_ts THEN $5 ELSE after_ts END, \
         months_total = months_done + $6, \
         walk_sport = CASE WHEN walk_sport = $7 THEN walk_sport ELSE NULL END \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome IS NULL";
