// ABOUTME: Request and response DTOs of the athlete's own plan routes under /api/me/plan (carnet#1841)
// ABOUTME: The read, what « Envoyer » would send and what it sent, a session's move, and the plan thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::CivilDate;
use pierre_core::models::{PlannedSession, PlannedSessionKind};
use pierre_services::plan_calendar_push::{KeyChange, PushReport, SkipReason};
use pierre_services::plan_card::PlanCard;
use pierre_tool_runtime::implementations::coach_session_editor::SessionExpect;
use pierre_tool_runtime::implementations::training_plan_push::{PendingSend, SendBlocked};
use serde::{Deserialize, Serialize};

use super::dto::ConversationResponse;

/// `GET /api/me/plan`: the athlete's own plan as their Plan tab reads it
/// (carnet#1841) — the live plan, their agent's unasked changes over it, and
/// what their « Envoyer » would send.
#[derive(Debug, Serialize)]
pub(super) struct OwnPlanResponse {
    /// The athlete's civil date both plans are projected on, `YYYY-MM-DD`.
    pub today: CivilDate,
    /// The live plan, as `GET /api/me/training-plan` shapes it; `null` when
    /// they have none.
    pub live: Option<PlanCard>,
    /// The overlay — the live weeks with the weeks their agent wrote unasked
    /// in their place — in the same shape; `null` when it holds nothing.
    pub draft: Option<PlanCard>,
    /// The dates whose sessions the overlay changes over the live plan,
    /// `YYYY-MM-DD`, sorted: the dates shown « Proposé ».
    pub proposed_dates: Vec<CivilDate>,
    /// The proposed dates grouped as they are accepted or refused: a moved
    /// session is `[from, to]`, every other date alone; each proposed date is
    /// in exactly one group.
    pub proposals: Vec<Vec<CivilDate>>,
    /// What « Envoyer » would send, judged from the ledger alone.
    pub pending_send: PendingSendResponse,
}

/// What the athlete's « Envoyer » would send, and whether they may.
#[derive(Debug, Serialize)]
pub(super) struct PendingSendResponse {
    /// Whether « Envoyer » is offered.
    pub send_available: bool,
    /// Why it is not: `no_provider`, `choose_calendar` or `coach_managed`;
    /// `null` when it is.
    pub reason: Option<&'static str>,
    /// The calendar backend it writes; `null` with no calendar to write or
    /// one still to choose.
    pub destination: Option<String>,
    /// With `choose_calendar`, the calendars to choose among.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<String>,
    /// One entry per calendar key it would create, update or withdraw.
    pub entries: Vec<PendingSendEntry>,
}

/// One calendar key « Envoyer » would change.
#[derive(Debug, Serialize)]
pub(super) struct PendingSendEntry {
    /// The entry's date, `YYYY-MM-DD`; a week note's is its week's start.
    pub date: CivilDate,
    /// `session` or `week_note`.
    pub kind: &'static str,
    /// `create`, `update` or `withdraw`.
    pub change: KeyChange,
    /// The entry as Dravr last wrote it; `null` for a create.
    pub before: Option<PlannedSession>,
    /// The entry as the plan wants it; `null` for a withdrawal.
    pub after: Option<PlannedSession>,
    /// The calendar backend it goes to.
    pub destination: String,
    /// Only in the send review, which reads the provider: whether the
    /// athlete changed the entry there after Dravr wrote it (« Modifiée sur
    /// Intervals.icu »). Such an entry is left alone unless replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_on_provider: Option<bool>,
}

impl PendingSendResponse {
    /// The pending send, as the wire carries it.
    pub(super) fn of(pending: PendingSend) -> Self {
        let destination = pending.destination.clone().unwrap_or_default();
        Self {
            send_available: pending.send_available(),
            reason: pending.blocked.map(SendBlocked::as_str),
            entries: pending
                .entries
                .into_iter()
                .map(|key| PendingSendEntry {
                    date: key.date,
                    kind: match key.kind {
                        PlannedSessionKind::Workout => "session",
                        PlannedSessionKind::WeekNote => "week_note",
                    },
                    change: key.change,
                    before: key.before,
                    after: key.after,
                    destination: destination.clone(),
                    edited_on_provider: None,
                })
                .collect(),
            destination: pending.destination,
            calendars: pending.calendars,
        }
    }
}

/// `POST /api/me/plan/send/review`: what « Envoyer » would send, checked
/// against the provider (carnet#1841).
#[derive(Debug, Serialize)]
pub(super) struct SendReviewResponse {
    /// The pending send, each entry saying whether the athlete edited it on
    /// the provider.
    pub pending_send: PendingSendResponse,
    /// The dates of the entries the athlete edited on the provider, sorted:
    /// left alone unless named in `replace_edited`.
    pub edited_on_provider: Vec<CivilDate>,
    /// How many dates the send changes as it stands, the edited ones left
    /// out: the N of « Envoyer (N) ».
    pub count: usize,
}

/// `POST /api/me/plan/send` body.
#[derive(Debug, Default, Deserialize)]
pub(super) struct SendRequest {
    /// The dates whose entry the athlete edited on the provider that the
    /// send overwrites anyway (« Remplacer »), `YYYY-MM-DD`.
    #[serde(default)]
    pub replace_edited: Vec<String>,
}

/// What the send created, updated and removed.
#[derive(Debug, Serialize)]
pub(super) struct SendCounts {
    /// Entries created on the calendar.
    pub created: usize,
    /// Entries updated in place.
    pub updated: usize,
    /// Entries taken off the calendar.
    pub removed: usize,
}

/// An entry the send left alone, and why.
#[derive(Debug, Serialize)]
pub(super) struct SendSkipped {
    /// The entry's date.
    pub date: CivilDate,
    /// Why: `edited_on_provider`.
    pub reason: SkipReason,
}

/// An entry the provider or the ledger refused.
#[derive(Debug, Serialize)]
pub(super) struct SendFailed {
    /// The entry's date.
    pub date: CivilDate,
    /// Why, as a stable code: `provider_refused` (the calendar answered with
    /// a refusal), `provider_unreachable` (it could not be reached), or
    /// `internal` (Dravr's own record of the entry). The full error stays in
    /// the server's log, never on the wire.
    pub reason: &'static str,
}

/// `POST /api/me/plan/send` answered 200: what the send did, and what is
/// still pending after it.
#[derive(Debug, Serialize)]
pub(super) struct SendResponse {
    /// What it created, updated and removed.
    pub report: SendCounts,
    /// The entries it left alone.
    pub skipped: Vec<SendSkipped>,
    /// The entries it failed to write.
    pub failed: Vec<SendFailed>,
    /// What « Envoyer » would send now.
    pub pending_send: PendingSendResponse,
}

impl SendResponse {
    /// The push's report and the pending send after it, as the wire carries
    /// them.
    pub(super) fn of(report: PushReport, pending_send: PendingSendResponse) -> Self {
        Self {
            report: SendCounts {
                created: report.created,
                updated: report.updated,
                removed: report.removed,
            },
            skipped: report
                .skipped
                .into_iter()
                .map(|entry| SendSkipped {
                    date: entry.date,
                    reason: entry.reason,
                })
                .collect(),
            failed: report
                .failed
                .into_iter()
                .map(|entry| SendFailed {
                    date: entry.date,
                    reason: entry.cause.as_str(),
                })
                .collect(),
            pending_send,
        }
    }
}

/// `POST /api/me/plan/days/{date}/{n}/move` body: session n as the client
/// last saw it, and the date it moves to (carnet#1841).
#[derive(Debug, Deserialize)]
pub(super) struct MoveSessionRequest {
    /// Session n as the card showed it.
    pub expect: SessionExpect,
    /// The date it moves to, `YYYY-MM-DD`.
    pub to_date: String,
}

/// `POST /api/me/plan/thread`: the caller's own plan thread, their Plan
/// tab's Dravr tab (carnet#1841).
#[derive(Debug, Serialize)]
pub(super) struct PlanThreadResponse {
    /// The thread.
    pub conversation: ConversationResponse,
    /// Whether this call opened it.
    pub opened: bool,
}
