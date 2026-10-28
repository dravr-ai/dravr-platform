// ABOUTME: A coach's after-the-fact review of the agent's group replies — the page of replies, a flag, a correction, the weekly count
// ABOUTME: One implementation behind the coach routes; flags and corrections are announced on the notify channel (carnet#847)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach review of group replies (carnet#847).
//!
//! Group replies go straight to the athletes: there is no review before send
//! and no mode switch (Phil, 2026-10-09). The coach's control is after the
//! fact, and this module is all of it:
//!
//! - [`list_replies`] — every reply the agent sent into a group the caller
//!   coaches, newest first, narrowed to one athlete or one group, each with
//!   its claim-verdict chips, the figures the checker could not settle
//!   (`unmatched_figures`) and the coach's own flags and corrections on it.
//! - [`flag_reply`] — a flag (wrong, unsafe or off-tone, with a note). It is
//!   the coach's alone: nothing the athlete reads carries it.
//! - [`correct_reply`] — a correction, posted into the group thread linked to
//!   the reply it corrects. The athlete reads it there.
//! - [`weekly_counts`] — the coach's flags and corrections per week, the
//!   "corrections needed" figure the cohort report prints.
//!
//! Who may review what is the repository's one rule
//! ([`pierre_database::repositories::coach_reply_reviews`]): the group's
//! attached coach, whatever tenant owns the group, on replies to a live member
//! who shares with them. The coach relationship is the scope, never the
//! caller's tenant: every account is its own tenant, so an athlete's group
//! that invited a coach lives in the athlete's tenant (carnet#1210). A flag or
//! a correction is written under the group's tenant. A reply outside it is
//! not found, so a coach cannot tell a reply to another coach's athlete from
//! one that does not exist.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc};
use pierre_contremaitre::EvidenceRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CoachReply, CoachReplyCorrection, CoachReplyFilter, CoachReplyFlag, CoachReplyReview,
    CoachReviewWeek, NewReplyCorrection, NewReplyFlag, ReplyFlagReason, ReplyReviewKind,
    ReplyReviewStamp, TenantId,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::{ClaimStatus, ClaimVerdict, VerdictLayer};
use pierre_providers::ai_scope;
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

use crate::chat_verdicts::{verdict_row, ChatVerdictRow};

/// Replies on one page when the caller names no size.
const DEFAULT_PAGE_SIZE: i64 = 20;

/// Most replies one page holds. Each reply's verdicts are one indexed read,
/// so the ceiling bounds the page's reads as well as its size.
const MAX_PAGE_SIZE: i64 = 50;

/// Longest note a flag keeps, in characters.
const MAX_FLAG_NOTE_CHARS: usize = 1000;

/// Longest correction, in characters — a chat message, not an essay.
const MAX_CORRECTION_CHARS: usize = 2000;

/// Weeks the weekly count covers when the caller names none.
const DEFAULT_WEEKS: u32 = 8;

/// Most weeks one weekly count covers: half a year, a cohort's span.
const MAX_WEEKS: u32 = 26;

/// A group the caller coaches, as the page's group filter offers it.
#[derive(Debug, Clone, Serialize)]
pub struct CoachedGroup {
    /// Group id
    pub id: String,
    /// Group name
    pub name: String,
}

/// An athlete in a group the caller coaches, as the athlete filter offers them.
#[derive(Debug, Clone, Serialize)]
pub struct CoachedAthlete {
    /// The athlete's user id — the filter's value
    pub user_id: String,
    /// Display name, else email
    pub name: Option<String>,
    /// Whether they share with the coach in at least one group. Replies to an
    /// athlete who shares in none are not listed.
    pub shares_with_coach: bool,
}

/// One reply as the coach's page shows it.
#[derive(Debug, Serialize)]
pub struct CoachReplyView {
    /// The reply's transcript entry id — what a flag or a correction names,
    /// and the page cursor
    pub id: String,
    /// The group it was sent into
    pub group_id: String,
    /// That group's name
    pub group_name: String,
    /// The athlete it answered
    pub athlete_user_id: String,
    /// That athlete's display name, else their email
    pub athlete_name: Option<String>,
    /// The reply's words; `None` when withheld from this caller
    pub content: Option<String>,
    /// The reply derives from first-party-only data and the caller is outside
    /// Dravr's apps (carnet#769): its words and verdicts are not served
    pub withheld: bool,
    /// When it was sent (RFC 3339)
    pub created_at: String,
    /// The claims the checker judged in it, in the athlete's chip shape
    pub verdicts: Vec<ChatVerdictRow>,
    /// Figures about the athlete the checker could neither match nor check
    /// (`unmatched_figures`) — the coach-only signal; the athlete sees no
    /// banner for them
    pub unmatched_figures: usize,
    /// The caller's flags on it, oldest first
    pub flags: Vec<CoachReplyFlag>,
    /// The caller's corrections on it, oldest first
    pub corrections: Vec<CoachReplyCorrection>,
}

/// One page of the coach's review.
#[derive(Debug, Serialize)]
pub struct CoachRepliesPage {
    /// Every active group the caller coaches, whatever tenant owns it
    pub groups: Vec<CoachedGroup>,
    /// Every live athlete of those groups, by name — the athlete filter
    pub athletes: Vec<CoachedAthlete>,
    /// The page's replies, newest first
    pub replies: Vec<CoachReplyView>,
    /// The cursor for the next (older) page; `None` when this one is the last
    pub next_before: Option<String>,
}

/// Which page of replies to read.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplyPageQuery {
    /// Only replies sent into this group
    pub group_id: Option<Uuid>,
    /// Only replies to this athlete
    pub athlete_user_id: Option<Uuid>,
    /// The oldest reply the caller already holds; `None` for the newest page
    pub before: Option<Uuid>,
    /// Page size, clamped to `1..=MAX_PAGE_SIZE`; `DEFAULT_PAGE_SIZE` when `None`
    pub limit: Option<i64>,
}

/// A flag, with the reply it was raised on.
#[derive(Debug, Clone)]
pub struct FlagOutcome {
    /// The reply
    pub reply: CoachReply,
    /// The flag
    pub flag: CoachReplyFlag,
}

/// A correction, with the reply it corrects.
#[derive(Debug, Clone)]
pub struct CorrectionOutcome {
    /// The reply
    pub reply: CoachReply,
    /// The correction, already in the group thread
    pub correction: CoachReplyCorrection,
}

/// How many of a reply's claims about the athlete state a figure the checker
/// could neither match to their records nor check against them.
///
/// The athlete-data layer reads such a claim as `unverifiable`: a figure
/// that matches no single synced activity, or one stated while nothing is
/// held for the window. `unverifiable` raises no banner for the athlete,
/// because a right weekly total misses every single activity too — so the
/// coach's page is where an invented figure surfaces (carnet#849). A claim
/// that states no figure at all ("makes no specific claim") is not counted:
/// there was nothing to match.
#[must_use]
fn unmatched_figures(verdicts: &[ClaimVerdict]) -> usize {
    verdicts
        .iter()
        .filter(|v| {
            v.layer_fired == VerdictLayer::AthleteData
                && v.status == ClaimStatus::Unverifiable
                && v.claim_text.chars().any(|c| c.is_ascii_digit())
        })
        .count()
}

/// The page of replies the caller reviews.
///
/// # Errors
///
/// Returns a repository error, including a stored conversation tenant that is
/// not a uuid.
pub async fn list_replies(
    repos: &RepositoryRegistry,
    corpus: &EvidenceRegistry,
    coach_user_id: Uuid,
    query: ReplyPageQuery,
) -> AppResult<CoachRepliesPage> {
    let (groups, athletes) = coached_roster(repos, coach_user_id).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let replies = repos
        .coach_reply_reviews
        .list_coach_replies(
            coach_user_id,
            &CoachReplyFilter {
                group_id: query.group_id,
                athlete_user_id: query.athlete_user_id,
                before: query.before,
                limit,
            },
        )
        .await?;

    let mut reviews = page_reviews(repos, coach_user_id, &replies).await?;
    // A full page may have more behind it; a short one is the last.
    let next_before = if i64::try_from(replies.len()).unwrap_or(i64::MAX) >= limit {
        replies.last().map(|r| r.entry_id.to_string())
    } else {
        None
    };

    let mut views = Vec::with_capacity(replies.len());
    for reply in replies {
        let (flags, corrections) = reviews.remove(&reply.entry_id).unwrap_or_default();
        views.push(reply_view(repos, corpus, reply, flags, corrections).await?);
    }

    Ok(CoachRepliesPage {
        groups,
        athletes,
        replies: views,
        next_before,
    })
}

/// Flag one reply the caller may review, and announce it.
///
/// # Errors
///
/// Returns not-found for a reply the caller may not review, invalid input for
/// a note longer than `MAX_FLAG_NOTE_CHARS`, and the repository error
/// otherwise.
pub async fn flag_reply(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    entry_id: Uuid,
    reason: ReplyFlagReason,
    note: Option<&str>,
) -> AppResult<FlagOutcome> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > MAX_FLAG_NOTE_CHARS) {
        return Err(AppError::invalid_input(format!(
            "A flag's note holds at most {MAX_FLAG_NOTE_CHARS} characters"
        )));
    }
    let reply = reviewable_reply(repos, coach_user_id, entry_id).await?;
    let flag = repos
        .coach_reply_reviews
        .record_flag(&NewReplyFlag {
            tenant_id: reply.group_tenant_id,
            group_id: reply.group_id,
            coach_user_id,
            athlete_user_id: reply.athlete_user_id,
            reply_entry_id: reply.entry_id,
            reason,
            note,
        })
        .await?;
    info!(
        target: "notify",
        event = "coach_reply.flagged",
        user_id = %coach_user_id,
        tenant_id = %reply.group_tenant_id,
        group_id = %reply.group_id,
        reason = %reason,
        "a coach flagged one of the agent's group replies"
    );
    Ok(FlagOutcome { reply, flag })
}

/// Post a correction to one reply the caller may review into the group
/// thread, linked to the reply, and announce it.
///
/// # Errors
///
/// Returns not-found for a reply the caller may not review, invalid input for
/// an empty correction or one longer than `MAX_CORRECTION_CHARS`, and the
/// repository error otherwise.
pub async fn correct_reply(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    entry_id: Uuid,
    content: &str,
) -> AppResult<CorrectionOutcome> {
    let content = content.trim();
    if content.is_empty() {
        return Err(AppError::invalid_input("A correction needs some words"));
    }
    if content.chars().count() > MAX_CORRECTION_CHARS {
        return Err(AppError::invalid_input(format!(
            "A correction holds at most {MAX_CORRECTION_CHARS} characters"
        )));
    }
    let reply = reviewable_reply(repos, coach_user_id, entry_id).await?;
    let correction = repos
        .coach_reply_reviews
        .record_correction(&NewReplyCorrection {
            tenant_id: reply.group_tenant_id,
            group_id: reply.group_id,
            coach_user_id,
            athlete_user_id: reply.athlete_user_id,
            reply_entry_id: reply.entry_id,
            content,
            transport_policy: reply.transport_policy,
        })
        .await?;
    info!(
        target: "notify",
        event = "coach_reply.corrected",
        user_id = %coach_user_id,
        tenant_id = %reply.group_tenant_id,
        group_id = %reply.group_id,
        "a coach corrected one of the agent's group replies"
    );
    Ok(CorrectionOutcome { reply, correction })
}

/// The caller's flags and corrections over the last `weeks` weeks, the
/// current one included, oldest week first. Weeks run Monday to Sunday, UTC;
/// a week with nothing in it is listed with zeros.
///
/// # Errors
///
/// Returns the repository error.
pub async fn weekly_counts(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    weeks: Option<u32>,
    now: DateTime<Utc>,
) -> AppResult<Vec<CoachReviewWeek>> {
    let weeks = weeks.unwrap_or(DEFAULT_WEEKS).clamp(1, MAX_WEEKS);
    let first_monday = week_start(now.date_naive()) - Duration::weeks(i64::from(weeks) - 1);
    let since = first_monday.and_time(NaiveTime::MIN).and_utc();
    let stamps = repos
        .coach_reply_reviews
        .list_review_stamps(coach_user_id, since)
        .await?;
    Ok(bucket_weeks(&stamps, first_monday, weeks))
}

/// The Monday of the week `day` falls in.
fn week_start(day: NaiveDate) -> NaiveDate {
    day - Duration::days(i64::from(day.weekday().num_days_from_monday()))
}

/// Count `stamps` into `weeks` consecutive weeks starting on `first_monday`.
/// A stamp before the first week or after the last is not counted.
fn bucket_weeks(
    stamps: &[ReplyReviewStamp],
    first_monday: NaiveDate,
    weeks: u32,
) -> Vec<CoachReviewWeek> {
    let mut buckets: BTreeMap<NaiveDate, CoachReviewWeek> = (0..weeks)
        .map(|offset| {
            let week_start = first_monday + Duration::weeks(i64::from(offset));
            (
                week_start,
                CoachReviewWeek {
                    week_start,
                    flags: 0,
                    corrections: 0,
                },
            )
        })
        .collect();
    for stamp in stamps {
        if let Some(bucket) = buckets.get_mut(&week_start(stamp.created_at.date_naive())) {
            match stamp.kind {
                ReplyReviewKind::Flag => bucket.flags += 1,
                ReplyReviewKind::Correction => bucket.corrections += 1,
            }
        }
    }
    buckets.into_values().collect()
}

/// The reply the caller asked for, when they may review it.
async fn reviewable_reply(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    entry_id: Uuid,
) -> AppResult<CoachReply> {
    repos
        .coach_reply_reviews
        .get_coach_reply(coach_user_id, entry_id)
        .await?
        .ok_or_else(|| AppError::not_found("Reply"))
}

/// The groups the caller coaches and their live athletes, by name. The coach
/// is never listed as their own athlete.
///
/// A group is the caller's by its `coach_user_id`, whatever tenant owns it:
/// every account is its own tenant, so an athlete's group that invited the
/// coach lives in the athlete's tenant, and keeping only the caller's tenant
/// hid it (carnet#1210). The coach relationship is the access key, as
/// [`pierre_database::repositories::CoachingGroupRepository::list_groups_coached_by`]
/// documents.
async fn coached_roster(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
) -> AppResult<(Vec<CoachedGroup>, Vec<CoachedAthlete>)> {
    let groups: Vec<_> = repos
        .groups
        .list_groups_coached_by(coach_user_id)
        .await?
        .into_iter()
        .filter(|g| g.is_active)
        .collect();

    let mut athletes: HashMap<Uuid, CoachedAthlete> = HashMap::new();
    for group in &groups {
        for member in repos.groups.list_members(&group.id.to_string()).await? {
            if member.user_id == coach_user_id {
                continue;
            }
            let entry = athletes
                .entry(member.user_id)
                .or_insert_with(|| CoachedAthlete {
                    user_id: member.user_id.to_string(),
                    name: member.display_name.clone(),
                    shares_with_coach: false,
                });
            entry.shares_with_coach |= member.coach_sharing_consent;
        }
    }
    let mut athletes: Vec<CoachedAthlete> = athletes.into_values().collect();
    athletes.sort_by(|a, b| {
        a.name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .cmp(&b.name.as_deref().unwrap_or_default().to_lowercase())
            .then_with(|| a.user_id.cmp(&b.user_id))
    });

    let groups = groups
        .into_iter()
        .map(|g| CoachedGroup {
            id: g.id.to_string(),
            name: g.name,
        })
        .collect();
    Ok((groups, athletes))
}

/// The caller's flags and corrections on the page's replies, by reply.
async fn page_reviews(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    replies: &[CoachReply],
) -> AppResult<HashMap<Uuid, (Vec<CoachReplyFlag>, Vec<CoachReplyCorrection>)>> {
    let (Some(newest), Some(oldest)) = (replies.first(), replies.last()) else {
        return Ok(HashMap::new());
    };
    let reviews = repos
        .coach_reply_reviews
        .list_reviews_for_window(coach_user_id, oldest.created_at, newest.created_at)
        .await?;
    // The window can hold replies the page's filter left out; only the page's
    // own are kept.
    let mut by_reply: HashMap<Uuid, (Vec<CoachReplyFlag>, Vec<CoachReplyCorrection>)> = replies
        .iter()
        .map(|r| (r.entry_id, (Vec::new(), Vec::new())))
        .collect();
    for review in reviews {
        if let Some((flags, corrections)) = by_reply.get_mut(&review.reply_entry_id()) {
            match review {
                CoachReplyReview::Flag(flag) => flags.push(flag),
                CoachReplyReview::Correction(correction) => corrections.push(correction),
            }
        }
    }
    Ok(by_reply)
}

/// One reply with its verdicts, shaped for the caller. A reply withheld from
/// an external caller keeps its place and loses its words and verdicts, which
/// quote it.
async fn reply_view(
    repos: &RepositoryRegistry,
    corpus: &EvidenceRegistry,
    reply: CoachReply,
    flags: Vec<CoachReplyFlag>,
    corrections: Vec<CoachReplyCorrection>,
) -> AppResult<CoachReplyView> {
    let admitted = ai_scope::admit_derived(reply.transport_policy);
    let verdicts = match (&reply.message_id, admitted) {
        (Some(message_id), true) => {
            let tenant = TenantId::parse_str(&reply.conversation_tenant_id).map_err(|e| {
                AppError::database(format!("Stored conversation tenant is not a uuid: {e}"))
            })?;
            let mut verdicts = repos
                .claim_verdicts
                .list_verdicts_for_message(tenant, message_id)
                .await?;
            ai_scope::retain_admitted(&mut verdicts, |v| v.transport_policy);
            verdicts
        }
        _ => Vec::new(),
    };
    Ok(CoachReplyView {
        id: reply.entry_id.to_string(),
        group_id: reply.group_id.to_string(),
        group_name: reply.group_name,
        athlete_user_id: reply.athlete_user_id.to_string(),
        athlete_name: reply.athlete_name,
        content: admitted.then_some(reply.content),
        withheld: !admitted,
        created_at: reply.created_at.to_rfc3339(),
        unmatched_figures: unmatched_figures(&verdicts),
        verdicts: verdicts
            .into_iter()
            .map(|v| verdict_row(v, corpus))
            .collect(),
        flags,
        corrections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::transport::TransportPolicy;
    use pierre_memory::{ClaimCategory, EvidenceStrength};

    fn verdict(layer: VerdictLayer, status: ClaimStatus, claim: &str) -> ClaimVerdict {
        ClaimVerdict {
            id: Uuid::new_v4().to_string(),
            tenant_id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            agent_id: None,
            conversation_id: None,
            message_id: Some("m1".to_owned()),
            claim_text: claim.to_owned(),
            category: ClaimCategory::AthleteData,
            status,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.0,
            layer_fired: layer,
            explanation: None,
            evidence_refs: None,
            created_at: Utc::now(),
            disposition: None,
            disposition_reason: None,
            disposition_note: None,
            disposed_by: None,
            disposed_at: None,
            transport_policy: TransportPolicy::AnyTransport,
        }
    }

    #[test]
    fn an_unsettled_figure_about_the_athlete_counts_and_nothing_else_does() {
        let verdicts = [
            // The connected athlete's invented marathon: the miss branch.
            verdict(
                VerdictLayer::AthleteData,
                ClaimStatus::Unverifiable,
                "your 2:41 marathon in May",
            ),
            // A claim naming no figure: nothing to match.
            verdict(
                VerdictLayer::AthleteData,
                ClaimStatus::Unverifiable,
                "you rode with the club",
            ),
            // A matched figure.
            verdict(
                VerdictLayer::AthleteData,
                ClaimStatus::Supported,
                "your 21 km run",
            ),
            // A contradicted figure already warns the athlete.
            verdict(
                VerdictLayer::AthleteData,
                ClaimStatus::Contradicted,
                "your 40 km ride",
            ),
            // A literature claim the evidence layer could not settle.
            verdict(
                VerdictLayer::Evidence,
                ClaimStatus::Unverifiable,
                "8 hours of sleep doubles recovery",
            ),
        ];
        assert_eq!(unmatched_figures(&verdicts), 1);
    }

    fn stamp(kind: ReplyReviewKind, at: &str) -> ReplyReviewStamp {
        ReplyReviewStamp {
            kind,
            created_at: DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn weeks_run_monday_to_sunday_and_empty_weeks_are_listed() {
        // Monday 2026-09-28 starts the first of three weeks.
        let first = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let stamps = [
            // Before the first week: not counted.
            stamp(ReplyReviewKind::Flag, "2026-09-27T23:59:59Z"),
            stamp(ReplyReviewKind::Flag, "2026-09-28T00:00:00Z"),
            stamp(ReplyReviewKind::Correction, "2026-10-04T23:59:59Z"),
            // The third week (Monday 2026-10-12): two flags, one correction.
            stamp(ReplyReviewKind::Flag, "2026-10-12T08:00:00Z"),
            stamp(ReplyReviewKind::Flag, "2026-10-15T08:00:00Z"),
            stamp(ReplyReviewKind::Correction, "2026-10-18T08:00:00Z"),
        ];
        let weeks = bucket_weeks(&stamps, first, 3);
        assert_eq!(
            weeks,
            vec![
                CoachReviewWeek {
                    week_start: first,
                    flags: 1,
                    corrections: 1
                },
                CoachReviewWeek {
                    week_start: NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
                    flags: 0,
                    corrections: 0
                },
                CoachReviewWeek {
                    week_start: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
                    flags: 2,
                    corrections: 1
                },
            ]
        );
    }

    #[test]
    fn a_week_starts_on_its_monday() {
        // Friday 2026-10-09 belongs to the week of Monday 2026-10-05.
        let friday = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        assert_eq!(
            week_start(friday),
            NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
        );
        let monday = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        assert_eq!(week_start(monday), monday);
    }
}
