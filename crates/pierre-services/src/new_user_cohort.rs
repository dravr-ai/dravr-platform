// ABOUTME: The new-user cohort report: each account created in a window, its milestones, and the funnel over them
// ABOUTME: Reads the signup cohort repository and does the per-user first-week arithmetic in Rust, not in either SQL engine

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How the accounts created in the last `N` days are doing.
//!
//! Counts come from Postgres, the system of record: complete for every
//! account whatever its analytics consent, with no identity to join and
//! nothing leaving the database. At alpha scale a row per account answers
//! "how are new users doing" better than a funnel percentage, so the report
//! carries both.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::{NewUserMilestones, NewUserTurn};
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use uuid::Uuid;

/// The window a report covers when the caller names none.
const DEFAULT_WINDOW_DAYS: i64 = 30;

/// The longest window a report covers.
const MAX_WINDOW_DAYS: i64 = 180;

/// The most accounts one report lists; a larger window is reported as
/// `truncated`, newest accounts kept.
const MAX_COHORT_USERS: usize = 2_000;

/// Days after sign-up that make an account's first week.
const FIRST_WEEK_DAYS: i64 = 7;

/// The new-user report for one window.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NewUserCohort {
    /// The window, in days back from `generated_at`.
    pub window_days: i64,
    /// When the report was computed.
    pub generated_at: DateTime<Utc>,
    /// Whether more accounts were created in the window than the report lists.
    pub truncated: bool,
    /// How many of the listed accounts reached each milestone.
    pub funnel: CohortFunnel,
    /// One row per account, newest first.
    pub users: Vec<NewUserRow>,
}

/// How many of the listed accounts reached each milestone.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct CohortFunnel {
    /// Accounts created in the window.
    pub signed_up: usize,
    /// Active, or approved by an operator.
    pub approved_or_active: usize,
    /// Proved their address.
    pub email_verified: usize,
    /// Connected a provider.
    pub provider_connected: usize,
    /// Had provider data arrive.
    pub first_sync: usize,
    /// Sent a first message.
    pub first_turn: usize,
    /// Old enough to be asked whether they came back after their first week:
    /// the denominator for `active_after_day_7`.
    pub day_7_eligible: usize,
    /// Of those, seen again after their first week.
    pub active_after_day_7: usize,
}

/// One account and how far it got.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NewUserRow {
    /// The account.
    pub user_id: String,
    /// The address it signs in with.
    pub email: String,
    /// The name it chose, if any.
    pub display_name: Option<String>,
    /// When it was created.
    pub signed_up_at: DateTime<Utc>,
    /// How it signed up: `email`, `google`, `apple`, …
    pub signup_source: String,
    /// `pending`, `active` or `suspended`.
    pub status: String,
    /// When it was approved or admitted.
    pub approved_at: Option<DateTime<Utc>>,
    /// Whether it proved its address.
    pub email_verified: bool,
    /// Its first provider connection.
    pub provider_connected_at: Option<DateTime<Utc>>,
    /// When provider data first arrived: the earlier of its first cached
    /// activity and its earliest completed health sync on record.
    pub first_sync_at: Option<DateTime<Utc>>,
    /// Its first message.
    pub first_turn_at: Option<DateTime<Utc>>,
    /// Messages it sent in its first week (so far, for an account younger
    /// than a week).
    pub turns_first_week: usize,
    /// When it was last seen.
    pub last_active_at: Option<DateTime<Utc>>,
}

/// `requested` days, defaulted and clamped to `1..=MAX_WINDOW_DAYS`.
#[must_use]
pub fn clamp_window_days(requested: Option<i64>) -> i64 {
    requested
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS)
}

/// The report for the `window_days` before `now`.
///
/// `scope` is the tenant the reader acts in, or `None` for a super-admin's
/// platform-wide read; every milestone is read in that same scope.
///
/// # Errors
///
/// Returns the repository's error when either read fails.
pub async fn fetch_new_user_cohort(
    repos: &RepositoryRegistry,
    window_days: i64,
    scope: Option<TenantId>,
    now: DateTime<Utc>,
) -> AppResult<NewUserCohort> {
    let since = now - Duration::days(window_days);
    // One more than listed tells whether the window holds more.
    let limit = i64::try_from(MAX_COHORT_USERS + 1).unwrap_or(i64::MAX);
    let (milestones, turns) = tokio::try_join!(
        repos.signup_cohort.new_user_milestones(since, scope, limit),
        repos.signup_cohort.new_user_turns(since, scope),
    )?;
    Ok(assemble_cohort(window_days, now, milestones, &turns))
}

/// Build the report from the repository's rows: per-account first-week
/// counts, the "came back after a week" test, and the funnel.
#[must_use]
fn assemble_cohort(
    window_days: i64,
    now: DateTime<Utc>,
    mut milestones: Vec<NewUserMilestones>,
    turns: &[NewUserTurn],
) -> NewUserCohort {
    let truncated = milestones.len() > MAX_COHORT_USERS;
    milestones.truncate(MAX_COHORT_USERS);

    let mut turns_by_user: HashMap<Uuid, Vec<DateTime<Utc>>> = HashMap::new();
    for turn in turns {
        turns_by_user
            .entry(turn.user_id)
            .or_default()
            .push(turn.sent_at);
    }

    let mut funnel = CohortFunnel::default();
    let users = milestones
        .into_iter()
        .map(|account| {
            let sent = turns_by_user
                .get(&account.user_id)
                .map_or(&[][..], Vec::as_slice);
            let first_week_end = account.signed_up_at + Duration::days(FIRST_WEEK_DAYS);
            let turns_first_week = sent
                .iter()
                .filter(|at| **at >= account.signed_up_at && **at < first_week_end)
                .count();
            let day_7_eligible = now >= first_week_end;
            let came_back = day_7_eligible
                && (account
                    .last_active_at
                    .is_some_and(|seen| seen >= first_week_end)
                    || sent.iter().any(|at| *at >= first_week_end));
            let first_sync_at = earliest(account.first_activity_at, account.first_health_sync_at);

            funnel.signed_up += 1;
            funnel.approved_or_active +=
                usize::from(account.status == "active" || account.approved_at.is_some());
            funnel.email_verified += usize::from(account.email_verified_at.is_some());
            funnel.provider_connected += usize::from(account.provider_connected_at.is_some());
            funnel.first_sync += usize::from(first_sync_at.is_some());
            funnel.first_turn += usize::from(account.first_turn_at.is_some());
            funnel.day_7_eligible += usize::from(day_7_eligible);
            funnel.active_after_day_7 += usize::from(came_back);

            NewUserRow {
                user_id: account.user_id.to_string(),
                email: account.email,
                display_name: account.display_name,
                signed_up_at: account.signed_up_at,
                signup_source: account.auth_provider,
                status: account.status,
                approved_at: account.approved_at,
                email_verified: account.email_verified_at.is_some(),
                provider_connected_at: account.provider_connected_at,
                first_sync_at,
                first_turn_at: account.first_turn_at,
                turns_first_week,
                last_active_at: account.last_active_at,
            }
        })
        .collect();

    NewUserCohort {
        window_days,
        generated_at: now,
        truncated,
        funnel,
        users,
    }
}

/// The earlier of two optional instants.
fn earliest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (one, None) | (None, one) => one,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0)
            .single()
            .unwrap_or_default()
    }

    fn account(user_id: Uuid, signed_up_at: DateTime<Utc>) -> NewUserMilestones {
        NewUserMilestones {
            user_id,
            email: "athlete@example.com".to_owned(),
            display_name: None,
            signed_up_at,
            auth_provider: "email".to_owned(),
            status: "pending".to_owned(),
            approved_at: None,
            email_verified_at: None,
            last_active_at: None,
            provider_connected_at: None,
            first_activity_at: None,
            first_health_sync_at: None,
            first_turn_at: None,
        }
    }

    #[test]
    fn window_days_default_and_clamp() {
        assert_eq!(clamp_window_days(None), DEFAULT_WINDOW_DAYS);
        assert_eq!(clamp_window_days(Some(0)), 1);
        assert_eq!(clamp_window_days(Some(-5)), 1);
        assert_eq!(
            clamp_window_days(Some(1)),
            1,
            "one day is the shortest window"
        );
        assert_eq!(clamp_window_days(Some(90)), 90);
        assert_eq!(clamp_window_days(Some(180)), 180, "180 days is kept");
        assert_eq!(clamp_window_days(Some(181)), 180, "181 days is cut to 180");
        assert_eq!(clamp_window_days(Some(10_000)), MAX_WINDOW_DAYS);
    }

    #[test]
    fn counts_first_week_turns_and_the_return_after_it() {
        let user = Uuid::new_v4();
        let mut row = account(user, at(1, 9));
        row.status = "active".to_owned();
        row.email_verified_at = Some(at(1, 10));
        row.provider_connected_at = Some(at(1, 11));
        row.first_activity_at = Some(at(2, 8));
        row.first_health_sync_at = Some(at(1, 12));
        row.first_turn_at = Some(at(1, 13));
        let turns = [
            NewUserTurn {
                user_id: user,
                sent_at: at(1, 13),
            },
            NewUserTurn {
                user_id: user,
                sent_at: at(7, 20),
            },
            // Day 9: after the first week ends (Oct 8, 09:00).
            NewUserTurn {
                user_id: user,
                sent_at: at(9, 8),
            },
        ];

        let cohort = assemble_cohort(30, at(20, 0), vec![row], &turns);

        let only = &cohort.users[0];
        assert_eq!(only.turns_first_week, 2);
        assert_eq!(
            only.first_sync_at,
            Some(at(1, 12)),
            "the earlier source wins"
        );
        assert!(only.email_verified);
        assert_eq!(
            cohort.funnel,
            CohortFunnel {
                signed_up: 1,
                approved_or_active: 1,
                email_verified: 1,
                provider_connected: 1,
                first_sync: 1,
                first_turn: 1,
                day_7_eligible: 1,
                active_after_day_7: 1,
            }
        );
    }

    #[test]
    fn a_week_old_account_is_not_yet_asked_whether_it_came_back() {
        let user = Uuid::new_v4();
        let mut row = account(user, at(18, 9));
        row.last_active_at = Some(at(19, 9));

        let cohort = assemble_cohort(30, at(20, 0), vec![row], &[]);

        assert_eq!(cohort.funnel.day_7_eligible, 0);
        assert_eq!(cohort.funnel.active_after_day_7, 0);
        assert_eq!(cohort.funnel.approved_or_active, 0, "still pending");
    }

    #[test]
    fn last_seen_after_the_first_week_counts_as_a_return() {
        let user = Uuid::new_v4();
        let mut row = account(user, at(1, 9));
        row.last_active_at = Some(at(12, 9));
        row.approved_at = Some(at(1, 10));

        let cohort = assemble_cohort(30, at(20, 0), vec![row], &[]);

        assert_eq!(cohort.funnel.active_after_day_7, 1);
        assert_eq!(cohort.funnel.approved_or_active, 1, "approved counts");
        assert_eq!(cohort.users[0].turns_first_week, 0);
    }

    #[test]
    fn another_users_turns_are_not_counted() {
        let user = Uuid::new_v4();
        let other = Uuid::new_v4();
        let turns = [NewUserTurn {
            user_id: other,
            sent_at: at(1, 12),
        }];

        let cohort = assemble_cohort(30, at(20, 0), vec![account(user, at(1, 9))], &turns);

        assert_eq!(cohort.users[0].turns_first_week, 0);
    }

    #[test]
    fn a_window_past_the_cap_is_reported_truncated() {
        let accounts = (0..=MAX_COHORT_USERS)
            .map(|_| account(Uuid::new_v4(), at(1, 9)))
            .collect();

        let cohort = assemble_cohort(30, at(20, 0), accounts, &[]);

        assert!(cohort.truncated);
        assert_eq!(cohort.users.len(), MAX_COHORT_USERS);
        assert_eq!(cohort.funnel.signed_up, MAX_COHORT_USERS);
    }
}
