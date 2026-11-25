// ABOUTME: The coach's morning brief: the athletes To handle holds, posted to the coach once each morning
// ABOUTME: Findings come from the roster read To handle is built from; one turn writes them, links and left-outs appended

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach morning brief (Routines P2, carnet#1791, ADR-036).
//!
//! Every morning the routine engine fires each coach's brief. It reads the
//! whole roster through the same code `GET /api/coach/roster` runs, under the
//! coach-read gate, and records the read on each sharer's who-read-my-data
//! log as the morning brief's. From each sharer's attention reasons it lists
//! the first one the coach has not marked Handled and has not switched off,
//! unless the coach left that athlete out of the brief; today's items come
//! before this week's. An athlete who doesn't share is never listed, only
//! counted.
//!
//! Deterministic code decides what is listed; one turn, charged to the coach,
//! only writes it up ([`KEY_PLATFORM_POST_MORNING_BRIEF_PROMPT`]). What must
//! reach the coach word for word is appended to the stored reply instead of
//! being left to the model: a link to each athlete in To handle, and how many
//! athletes the brief could not judge — they don't share, or their data could
//! not be read — so that a missing signal never reads as "all fine".
//!
//! Nothing listed, nothing posted: the run is recorded as empty, and the
//! brief's control in To handle says it ran with nothing to report.

use std::collections::BTreeMap;
use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_contremaitre::messaging_strings::{
    format_template, KEY_PLATFORM_POST_MORNING_BRIEF_LINKS,
    KEY_PLATFORM_POST_MORNING_BRIEF_NOT_SHARING, KEY_PLATFORM_POST_MORNING_BRIEF_PROMPT,
    KEY_PLATFORM_POST_MORNING_BRIEF_UNREADABLE,
};
use pierre_core::errors::AppError;
use pierre_core::models::{BriefSettings, LeftOut, PostPurpose, Routine, RoutineKind, TenantId};
use pierre_core::transport::Transport;
use pierre_database::repositories::{CoachReadSurface, ListedItem};
use pierre_providers::ai_scope::{self, Provenance};
use pierre_services::coach_attention::{AttentionHorizon, AttentionKind};
use pierre_services::coach_roster::{roster_access, RosterAccess, RosterAthleteStatus};
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::coach_roster::{RosterReader, RosterRow, RosterWindow};
use crate::services::check_in_post::RoutinePoster;
use crate::services::platform_post::{PostAccess, PostSurface};
use crate::services::post_delivery::{
    linked_dms, post_to_one, IntentFindings, PostAthlete, PostIntent,
};

/// What one firing of a coach's brief did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BriefRun {
    /// The brief's reply was stored on that surface; it listed `items`.
    Posted {
        /// Where it landed.
        surface: PostSurface,
        /// The athletes it listed, each with the reason it listed them for.
        items: Vec<ListedItem>,
        /// The athletes it could not judge, named in its footer.
        left_out: LeftOut,
    },
    /// Its turn ran on that surface and the reply was withheld.
    Withheld {
        /// Where its turn ran.
        surface: PostSurface,
        /// The athletes it could not judge.
        left_out: LeftOut,
    },
    /// No athlete needed listing: nothing was posted. Those it could not
    /// judge are still counted, for the run to name.
    Empty(LeftOut),
    /// Its owner no longer reads a roster (not a coach, or coach access off).
    NotACoach,
    /// This slot's brief was already claimed — posted, withheld or running.
    AlreadyClaimed,
    /// Nothing could be attempted: the roster was unreadable, or no thread or
    /// instruction could be found. The day's claim, if taken, was released.
    Unattempted,
}

/// One athlete the brief lists.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Finding {
    /// The athlete.
    pub athlete: Uuid,
    /// Their name, as To handle shows it.
    pub name: String,
    /// The reason they are listed for: their first reason still open.
    pub kind: AttentionKind,
    /// Whether it is for today or this week.
    pub horizon: AttentionHorizon,
    /// The reason's figures (days quiet, provider, ...), as To handle reads them.
    pub params: BTreeMap<&'static str, Value>,
}

/// What the brief found on a coach's roster.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct BriefFindings {
    /// The athletes it lists: today's first, then this week's, each in
    /// roster order.
    pub items: Vec<Finding>,
    /// Athletes left out because they don't share with the coach, or the
    /// coach-read gate refused their read.
    pub not_sharing: usize,
    /// Sharers whose training could not be read this morning.
    pub unreadable: usize,
}

impl BriefFindings {
    /// The athletes the brief could not judge, as a run records them.
    pub(crate) fn left_out(&self) -> LeftOut {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        LeftOut {
            not_sharing: count(self.not_sharing),
            unreadable: count(self.unreadable),
        }
    }

    /// The items as a run records them.
    pub(crate) fn listed(&self) -> Vec<ListedItem> {
        self.items
            .iter()
            .map(|finding| ListedItem {
                athlete_user_id: finding.athlete,
                reason: finding.kind.as_str().to_owned(),
            })
            .collect()
    }
}

/// What the brief lists from `rows`, the roster as To handle reads it, given
/// the sharers the coach-read gate `refused` and what the coach left out.
///
/// Only an athlete who shares with the coach is listed, and so reaches the
/// coach's model. One who doesn't share — never did, or stopped, though To
/// handle shows the stop as an item — or whom the gate refused is counted as
/// not sharing (Phil, 2026-11-25: the issue's "lists only consented
/// athletes and names the rest"). A sharer with nothing listed and nothing
/// of their training read is counted as unreadable. One who shares with no
/// source connected was read: there is nothing to read, which is not a
/// failure.
pub(crate) fn brief_findings(
    rows: &[RosterRow],
    refused: &[Uuid],
    settings: &BriefSettings,
) -> BriefFindings {
    let mut found = BriefFindings::default();
    for row in rows.iter().filter(|row| settings.lists(row.user_id)) {
        if row.status == RosterAthleteStatus::NoCoachConsent || refused.contains(&row.user_id) {
            found.not_sharing += 1;
            continue;
        }
        let lead = row
            .attention
            .iter()
            .find(|reason| reason.dismissed_at.is_none() && settings.checks(reason.kind.as_str()));
        if let Some(reason) = lead {
            found.items.push(Finding {
                athlete: row.user_id,
                name: row.name.clone(),
                kind: reason.kind,
                horizon: reason.horizon,
                params: reason.params.clone(),
            });
        } else if row.status == RosterAthleteStatus::Shared && row.training.is_none() {
            found.unreadable += 1;
        }
    }
    // Stable: within a horizon the roster's own order holds.
    found.items.sort_by_key(|finding| finding.horizon);
    found
}

/// The findings as the prompt carries them, one athlete per line: name,
/// horizon, reason and its figures. The model writes them up in the coach's
/// language; it is told to add nothing they don't carry.
pub(crate) fn findings_lines(findings: &BriefFindings) -> String {
    findings
        .items
        .iter()
        .map(|finding| {
            let horizon = match finding.horizon {
                AttentionHorizon::Today => "today",
                AttentionHorizon::Week => "this week",
            };
            let figures: Vec<String> = finding
                .params
                .iter()
                .filter(|(name, _)| **name != "name")
                .map(|(name, value)| match value {
                    Value::String(text) => format!("{name}: {text}"),
                    other => format!("{name}: {other}"),
                })
                .collect();
            let line = format!("- {} · {horizon} · {}", finding.name, finding.kind.as_str());
            if figures.is_empty() {
                line
            } else {
                format!("{line} · {}", figures.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the brief appends to its reply in `locale`: a link to each listed
/// athlete in To handle (when the app's address is configured), then how many
/// athletes it left out and why.
pub(crate) fn appendix(
    resources: &ServerContext,
    findings: &BriefFindings,
    frontend_url: Option<&str>,
    locale: &str,
) -> String {
    let strings = &resources.mcp.messaging_strings_registry;
    let mut lines = Vec::new();
    if let Some(base) = frontend_url.map(|url| url.trim_end_matches('/')) {
        lines.push(strings.get(KEY_PLATFORM_POST_MORNING_BRIEF_LINKS, locale));
        for finding in &findings.items {
            // A name is the link's text: brackets would end it early.
            let name: String = finding
                .name
                .chars()
                .filter(|c| !matches!(c, '[' | ']'))
                .collect();
            lines.push(format!(
                "- [{name}]({base}/#home/athlete/{})",
                finding.athlete
            ));
        }
    }
    let left_out = [
        (
            findings.not_sharing,
            KEY_PLATFORM_POST_MORNING_BRIEF_NOT_SHARING,
        ),
        (
            findings.unreadable,
            KEY_PLATFORM_POST_MORNING_BRIEF_UNREADABLE,
        ),
    ];
    let footer: Vec<String> = left_out
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, key)| format_template(&strings.get(key, locale), &[&count.to_string()]))
        .collect();
    if !footer.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(footer);
    }
    lines.join("\n")
}

/// How long after a brief listed an athlete the coach's action on them still
/// counts as acting on the brief: the pilot's measure (carnet#1791).
pub(crate) const ACTED_WITHIN: Elapsed = Elapsed::hours(24);

/// How the coach acted on an athlete a brief listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActedOn {
    /// They marked the athlete's item Handled in To handle.
    Treated,
    /// They opened the athlete.
    Opened,
}

impl ActedOn {
    /// The action as `routine.acted` and the run item record it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Treated => "treated",
            Self::Opened => "opened",
        }
    }
}

/// Record that `coach` acted on `athlete`, the first time after a brief in the
/// last [`ACTED_WITHIN`] listed them: each listed item counts once, as
/// `routine.acted`. Logged when it cannot be recorded: the action itself
/// already happened.
pub(crate) async fn record_acted(
    resources: &ServerContext,
    coach: Uuid,
    athlete: Uuid,
    action: ActedOn,
) {
    let Some(home) = coach_home(resources, coach).await else {
        return;
    };
    let now = Timestamp::now();
    let marked = resources
        .common
        .repos
        .routines
        .mark_listed_acted(
            home,
            coach,
            athlete,
            now - ACTED_WITHIN,
            action.as_str(),
            now,
        )
        .await;
    match marked {
        Ok(reasons) => {
            for reason in reasons {
                report_acted(coach, home, action, &reason);
            }
        }
        Err(e) => warn!(error = %e, "Morning brief: an action on a listed athlete went uncounted"),
    }
}

/// The coach's home tenant, where their brief lives; `None` (logged when
/// unreadable) when they hold none.
async fn coach_home(resources: &ServerContext, coach: Uuid) -> Option<TenantId> {
    match resources.common.repos.tenants.first_for_user(coach).await {
        Ok(home) => home,
        Err(e) => {
            warn!(error = %e, "Morning brief: the coach's tenant is unreadable; an action went uncounted");
            None
        }
    }
}

/// Report one listed item the coach acted on.
fn report_acted(coach: Uuid, home: TenantId, action: ActedOn, reason: &str) {
    info!(
        target: "notify",
        event = "routine.acted",
        user_id = %coach,
        tenant_id = %home,
        kind = RoutineKind::MorningBrief.as_str(),
        action = action.as_str(),
        reason,
        "routine item acted on"
    );
}

/// Fire `routine`, a coach's brief, at `now` on the coach's clock `zone`:
/// read the roster, and when anyone needs listing claim the day, post, and
/// mark the day posted.
pub(crate) async fn run_morning_brief(
    resources: &Arc<ServerContext>,
    poster: RoutinePoster<'_>,
    routine: &Routine,
    zone: &Zone,
    now: Timestamp,
) -> BriefRun {
    let provenance = Provenance::new();
    let findings = match read_findings(resources, routine, &provenance).await {
        Ok(findings) => findings,
        Err(run) => return run,
    };
    if findings.items.is_empty() {
        info!(
            routine_id = %routine.id,
            not_sharing = findings.not_sharing,
            unreadable = findings.unreadable,
            "Morning brief: nothing to report"
        );
        return BriefRun::Empty(findings.left_out());
    }
    let day = match DayClaim::take(resources, routine, zone, now).await {
        Ok(day) => day,
        Err(run) => return run,
    };
    let Some((surface, stored)) =
        post_brief(resources, poster, routine, &findings, &provenance, now).await
    else {
        day.release().await;
        return BriefRun::Unattempted;
    };
    if !stored {
        return BriefRun::Withheld {
            surface,
            left_out: findings.left_out(),
        };
    }
    day.mark_posted(now).await;
    BriefRun::Posted {
        surface,
        items: findings.listed(),
        left_out: findings.left_out(),
    }
}

/// What the brief lists on `routine`'s owner's roster, read into
/// `provenance`; the run to report instead when there is no roster to read.
///
/// The findings go into a model's prompt: the roster is read as a model reads
/// it, over the turn's own transport, and what it served — and whose data
/// each listed line is — is tracked so the turn starts from it and stamps the
/// stored brief with it (carnet#769, carnet#1174).
async fn read_findings(
    resources: &Arc<ServerContext>,
    routine: &Routine,
    provenance: &Provenance,
) -> Result<BriefFindings, BriefRun> {
    let coach = routine.owner_user_id;
    let groups = match roster_access(&resources.common.repos, coach).await {
        Ok(RosterAccess::Granted(groups)) => groups,
        Ok(RosterAccess::NoCoachAccess(_) | RosterAccess::NotACoach) => {
            return Err(BriefRun::NotACoach)
        }
        Err(e) => {
            warn!(error = %e, "Morning brief: the coach's groups are unreadable");
            return Err(BriefRun::Unattempted);
        }
    };
    let reader = RosterReader {
        coach,
        outside: false,
        surface: CoachReadSurface::MorningBrief,
        for_model: true,
    };
    let read = async {
        let roster = reader.read(resources, &groups, RosterWindow::ALL).await?;
        let findings = brief_findings(&roster.athletes, &roster.refused, &routine.brief);
        for finding in &findings.items {
            ai_scope::note_member_read(finding.athlete);
        }
        Ok::<_, AppError>(findings)
    };
    let tracked = ai_scope::tracking(provenance.clone(), read);
    ai_scope::serve_over(Transport::PlatformJob, tracked)
        .await
        .map_err(|e| {
            warn!(error = %e, "Morning brief: the roster is unreadable");
            BriefRun::Unattempted
        })
}

/// Post the brief of `findings` to `routine`'s owner: on the DM they last
/// wrote from while its window is open, else in the app with a push.
async fn post_brief(
    resources: &Arc<ServerContext>,
    poster: RoutinePoster<'_>,
    routine: &Routine,
    findings: &BriefFindings,
    provenance: &Provenance,
    now: Timestamp,
) -> Option<(PostSurface, bool)> {
    let recipient = PostAthlete {
        user_id: routine.owner_user_id,
        tenant_id: routine.tenant_id,
    };
    let lines = findings_lines(findings);
    let strings = &resources.mcp.messaging_strings_registry;
    let prompt = |locale: &str| {
        let template = strings.get(KEY_PLATFORM_POST_MORNING_BRIEF_PROMPT, locale);
        (!template.trim().is_empty()).then(|| format_template(&template, &[&lines]))
    };
    let frontend_url = resources.common.config.frontend_url.as_deref();
    let appended = |locale: &str| appendix(resources, findings, frontend_url, locale);
    let dms = linked_dms(resources, recipient, now).await;
    let intent = PostIntent {
        purpose: PostPurpose::Routine,
        access: PostAccess::SuggestOnly,
        findings: Some(IntentFindings {
            appendix: &appended,
            provenance,
        }),
        push: true,
    };
    post_to_one(
        resources,
        poster.post,
        poster.resolver,
        recipient,
        dms,
        &prompt,
        intent,
    )
    .await
}

/// The claim on one coach's brief for one local day: one slot never posts
/// twice, however many wakes deliver it.
struct DayClaim<'a> {
    resources: &'a ServerContext,
    tenant: TenantId,
    coach: Uuid,
    /// The routine's id: one claim per brief per day.
    subject: String,
    lease: String,
}

impl<'a> DayClaim<'a> {
    /// Claim `routine`'s day on the coach's clock `zone`; the run to report
    /// when the day is already claimed or the claim failed.
    async fn take(
        resources: &'a ServerContext,
        routine: &Routine,
        zone: &Zone,
        now: Timestamp,
    ) -> Result<DayClaim<'a>, BriefRun> {
        // The coach's local midnight: a claim taken before it was an earlier day's.
        let Ok(since) = zone.start_of_day(zone.date_at(now)) else {
            warn!("Morning brief: the coach's local midnight is out of range");
            return Err(BriefRun::Unattempted);
        };
        let subject = routine.id.to_string();
        let claimed = resources
            .common
            .repos
            .platform_post_claims
            .claim_platform_post_since(
                routine.tenant_id,
                routine.owner_user_id,
                PostPurpose::Routine.as_str(),
                &subject,
                since,
                now,
            )
            .await;
        match claimed {
            Ok(Some(lease)) => Ok(DayClaim {
                resources,
                tenant: routine.tenant_id,
                coach: routine.owner_user_id,
                subject,
                lease,
            }),
            Ok(None) => Err(BriefRun::AlreadyClaimed),
            Err(e) => {
                warn!(error = %e, "Morning brief: the day's claim failed");
                Err(BriefRun::Unattempted)
            }
        }
    }

    /// Release the day: nothing could be attempted, so a later wake may try.
    async fn release(self) {
        let released = self
            .resources
            .common
            .repos
            .platform_post_claims
            .release_platform_post(
                self.tenant,
                self.coach,
                PostPurpose::Routine.as_str(),
                &self.subject,
                &self.lease,
            )
            .await;
        if let Err(e) = released {
            warn!(error = %e, "Morning brief: releasing the claim of an unmade brief failed");
        }
    }

    /// Mark the day posted: the brief landed, and the daily cap counts it.
    async fn mark_posted(self, now: Timestamp) {
        let marked = self
            .resources
            .common
            .repos
            .platform_post_claims
            .mark_platform_post_posted(
                self.tenant,
                self.coach,
                PostPurpose::Routine.as_str(),
                &self.subject,
                &self.lease,
                now,
            )
            .await;
        match marked {
            Ok(true) => {}
            Ok(false) => warn!("Morning brief: the claim no longer held the day's row"),
            Err(e) => warn!(error = %e, "Morning brief: marking the day posted failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_services::coach_attention::{AttentionAction, AttentionReason, AttentionTarget};
    use serde_json::json;

    use super::*;

    fn reason(kind: AttentionKind, horizon: AttentionHorizon) -> AttentionReason {
        AttentionReason {
            id: kind.as_str().to_owned(),
            kind,
            horizon,
            rank: 0,
            key: "coachHome.reason.test",
            params: BTreeMap::from([("days", json!(9))]),
            evidence: Vec::new(),
            action: AttentionAction::WriteToAthlete,
            target: AttentionTarget::default(),
            fingerprint: String::new(),
            dismissed_at: None,
        }
    }

    fn row(name: &str, status: RosterAthleteStatus, attention: Vec<AttentionReason>) -> RosterRow {
        RosterRow {
            user_id: Uuid::new_v4(),
            name: name.to_owned(),
            groups: Vec::new(),
            status,
            sharing_stopped_at: None,
            training: None,
            plan: None,
            plan_thread: None,
            agent_replies: None,
            season: None,
            attention,
        }
    }

    #[test]
    fn today_comes_before_this_week_and_each_athlete_is_listed_once() {
        let week = row(
            "Wes",
            RosterAthleteStatus::Shared,
            vec![reason(AttentionKind::PlanEnded, AttentionHorizon::Week)],
        );
        let today = row(
            "Tia",
            RosterAthleteStatus::Shared,
            vec![
                reason(AttentionKind::NothingReceived, AttentionHorizon::Today),
                reason(AttentionKind::PlanEnded, AttentionHorizon::Week),
            ],
        );
        let found = brief_findings(&[week, today], &[], &BriefSettings::default());
        let names: Vec<&str> = found.items.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            ["Tia", "Wes"],
            "bug: today's items lead, one line per athlete"
        );
        assert_eq!(
            found.items[0].kind,
            AttentionKind::NothingReceived,
            "bug: an athlete is listed for their first open reason"
        );
    }

    #[test]
    fn a_handled_reason_gives_way_to_the_next_and_a_muted_athlete_is_never_listed() {
        let mut handled = reason(AttentionKind::NothingReceived, AttentionHorizon::Today);
        handled.dismissed_at = Some(Timestamp::now());
        let partly = row(
            "Pat",
            RosterAthleteStatus::Shared,
            vec![
                handled,
                reason(AttentionKind::PlanEnded, AttentionHorizon::Week),
            ],
        );
        let muted = row(
            "Mo",
            RosterAthleteStatus::Shared,
            vec![reason(
                AttentionKind::NothingReceived,
                AttentionHorizon::Today,
            )],
        );
        let settings = BriefSettings {
            reasons_off: Vec::new(),
            muted: vec![muted.user_id],
        };
        let found = brief_findings(&[partly, muted], &[], &settings);
        assert_eq!(
            found.items.len(),
            1,
            "bug: the muted athlete was listed: {found:?}"
        );
        assert_eq!(
            found.items[0].kind,
            AttentionKind::PlanEnded,
            "bug: a reason marked Handled was listed"
        );
    }

    #[test]
    fn a_reason_switched_off_is_not_checked() {
        let quiet = row(
            "Quinn",
            RosterAthleteStatus::Shared,
            vec![reason(
                AttentionKind::NothingReceived,
                AttentionHorizon::Today,
            )],
        );
        let settings = BriefSettings {
            reasons_off: vec!["nothing_received".to_owned()],
            muted: Vec::new(),
        };
        let found = brief_findings(&[quiet], &[], &settings);
        assert!(
            found.items.is_empty(),
            "bug: a switched-off reason was listed: {found:?}"
        );
    }

    #[test]
    fn athletes_the_brief_could_not_judge_are_counted_never_dropped() {
        // Stopped sharing: To handle lists the stop, the brief only counts it.
        let stopped = row(
            "Sam",
            RosterAthleteStatus::NoCoachConsent,
            vec![reason(
                AttentionKind::ConsentRevoked,
                AttentionHorizon::Today,
            )],
        );
        let refused = row("Rae", RosterAthleteStatus::Shared, Vec::new());
        let unread = row("Uma", RosterAthleteStatus::Shared, Vec::new());
        let no_source = row("Nia", RosterAthleteStatus::NoSource, Vec::new());
        let found = brief_findings(
            &[stopped, refused.clone(), unread, no_source],
            &[refused.user_id],
            &BriefSettings::default(),
        );
        assert_eq!(
            found.not_sharing, 2,
            "bug: consent off and a refused read are not sharing"
        );
        assert_eq!(
            found.unreadable, 1,
            "bug: a sharer with nothing read is unreadable"
        );
        assert!(
            found.items.is_empty(),
            "bug: an athlete who does not share reached the prompt: {found:?}"
        );
    }

    #[test]
    fn a_finding_line_carries_its_figures_and_never_the_name_twice() {
        let mut quiet = reason(AttentionKind::NothingReceived, AttentionHorizon::Today);
        quiet.params.insert("name", json!("Tia"));
        quiet.params.insert("provider", json!("Strava"));
        let found = brief_findings(
            &[row("Tia", RosterAthleteStatus::Shared, vec![quiet])],
            &[],
            &BriefSettings::default(),
        );
        assert_eq!(
            findings_lines(&found),
            "- Tia · today · nothing_received · days: 9, provider: Strava",
            "bug: the line the model reads changed shape"
        );
    }
}
