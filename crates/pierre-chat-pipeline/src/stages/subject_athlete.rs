// ABOUTME: A human coach's thread bound to one athlete — the coach gates re-run every turn, then the athlete's coach view
// ABOUTME: Refused, one line and nothing of the athlete; granted, how the thread works, their roster line, plan, draft and linked wellness
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The athlete a coach's thread works on (carnet#1070)
//!
//! A group's human coach opens one thread per coached athlete from Group
//! info. Every turn in it is about that athlete, never about the coach who
//! writes it, so prompt assembly skips the sender's own context — their
//! connected providers, dossier, playbooks, notes, followups, commitments and
//! plan — and runs this stage in its place.
//!
//! The coaching gates run again on every turn
//! ([`coached_athlete_access`]): a consent withdrawn, a member who left, a
//! coach detached or an athlete now in two tenants ends the coach's access on
//! the next turn. Refused, the prompt carries one line saying so and why, and
//! nothing of the athlete. Granted, it carries the coach's view of them:
//!
//! - who the athlete is, that the thread is the coach's private workspace for
//!   their plan, and how drafts, publishing and conflicts work;
//! - their roster line, projected exactly as `get_roster_overview` lists it,
//!   so form arrives banded as a share of their own fitness;
//! - their live season and the coach's draft, with where the draft stands;
//! - their wellness, only what the coach's link filed under the athlete's
//!   `wellness_read` consent ([`linked_wellness`]).
//!
//! Never their dossier, memory, notes, followups or commitments, and never
//! another member's data. The plan block that follows (stage 7f.2) is the
//! athlete's too: the coach's draft over their live weeks, or their live
//! season ([`GrantedSubject::plan_shown`]).
//!
//! What the thread says is filed on nobody's account ([`files_on_sender`]):
//! no extracted fact or playbook advice, and no tool that writes the coach's
//! own data (`TurnSeat::AthleteThread`). The plan tools are the one write, and
//! they write the athlete's draft.

use std::fmt::Write as _;

use chrono::{NaiveDate, Utc};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{ConversationSubject, TenantId};
use pierre_core::untrusted::{display_line, ROSTER_NAME_MAX_CHARS};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::ConversationRecord;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use pierre_providers::backend_resolver::brand_name;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{coach_plan_view, CoachDraftTarget, CoachPlanView};
use pierre_services::coached_athlete::{
    coached_athlete_access, CoachedAthleteAccess, CoachedAthleteRefusal,
};
use pierre_services::linked_wellness::{linked_wellness, LinkedWellness, LINKED_WELLNESS_DAYS};
use tracing::{info, warn};
use uuid::Uuid;

use crate::ChatPipelineContext;

/// Heads the block, granted or refused.
const HEADING: &str = "\n\n## The athlete this thread works on\n\n";

/// Why the coach may not act for the thread's athlete on this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectRefusal {
    /// A coaching gate refused.
    Gate(CoachedAthleteRefusal),
    /// The gates could not be asked: the sender's id does not parse, or a
    /// repository read failed. Nothing of the athlete is shown.
    Unconfirmed,
}

impl SubjectRefusal {
    /// The one line a refused turn carries. The two gates that fail before
    /// the athlete is shown to be the coach's name nobody.
    fn line(&self) -> String {
        let why = match self {
            Self::Gate(CoachedAthleteRefusal::NotGroupCoach) => {
                "the person writing no longer coaches the group they coached this athlete in"
                    .to_owned()
            }
            Self::Gate(CoachedAthleteRefusal::NotAMember) => {
                "this athlete is no longer a member of the group they were coached in".to_owned()
            }
            Self::Gate(CoachedAthleteRefusal::CoachSharingRevoked { athlete, group }) => format!(
                "{} has stopped sharing their data with the coach of '{}' — they can share it \
                 again with `/group consent coach yes`",
                name(athlete),
                name(group)
            ),
            Self::Gate(CoachedAthleteRefusal::NoTenant { athlete }) => format!(
                "{} belongs to no tenant, so there is nowhere to keep their plan",
                name(athlete)
            ),
            Self::Gate(CoachedAthleteRefusal::SeveralTenants { athlete }) => format!(
                "{} belongs to several tenants, so their plan would show on one of their \
                 surfaces and be missing on another",
                name(athlete)
            ),
            Self::Unconfirmed => {
                "the coach's access to this athlete could not be confirmed on this turn".to_owned()
            }
        };
        format!(
            "The coach can no longer act for this thread's athlete: {why}. Say so plainly, and \
             read, plan or save nothing for them here."
        )
    }
}

/// The coach may act for the thread's athlete on this turn, and what of
/// their plan the turn shows.
#[derive(Debug)]
pub struct GrantedSubject {
    access: CoachedAthleteAccess,
    coach: Uuid,
    /// The athlete's live season and the coach's draft over it; `None` when
    /// they could not be read on this turn (logged).
    view: Option<CoachPlanView>,
    today: NaiveDate,
}

impl GrantedSubject {
    /// The athlete's one home tenant, where their plan lives.
    pub const fn tenant(&self) -> TenantId {
        self.access.tenant
    }

    /// The athlete.
    pub const fn athlete(&self) -> Uuid {
        self.access.athlete_user_id
    }

    /// The athlete's civil date.
    pub const fn today(&self) -> NaiveDate {
        self.today
    }

    /// The plan the turn's plan block shows: the coach's draft over the
    /// athlete's live weeks when the coach holds one, otherwise the
    /// athlete's live season; `None` with neither.
    pub fn plan_shown(&self) -> Option<(&TrainingPlan, &[PlanWeek])> {
        let view = self.view.as_ref()?;
        match (&view.draft, &view.live) {
            (Some(draft), _) => Some((&draft.plan, draft.weeks.as_slice())),
            (None, Some(live)) => Some((&live.plan, live.weeks.as_slice())),
            (None, None) => None,
        }
    }
}

/// What a turn in a coach's thread about one athlete may show of them.
#[derive(Debug)]
pub enum SubjectTurn {
    /// The gates let the coach act for the athlete.
    Granted(Box<GrantedSubject>),
    /// They no longer do.
    Refused(SubjectRefusal),
}

/// Whether `sender` may act for `subject`'s athlete, asked of the coaching
/// gates now. A repository failure refuses: nothing of the athlete is shown
/// on a turn whose access could not be confirmed.
pub async fn subject_access(
    repos: &RepositoryRegistry,
    subject: ConversationSubject,
    sender: &str,
) -> Result<(CoachedAthleteAccess, Uuid), SubjectRefusal> {
    let Ok(coach) = parse_uuid(sender) else {
        return Err(SubjectRefusal::Unconfirmed);
    };
    match coached_athlete_access(repos, coach, subject.athlete_user_id, subject.group_id).await {
        Ok(Ok(access)) => Ok((access, coach)),
        Ok(Err(refused)) => {
            info!(
                athlete = %subject.athlete_user_id,
                group_id = %subject.group_id,
                reason = refused.reason(),
                "coach's athlete thread: the coaching gates refused this turn"
            );
            Err(SubjectRefusal::Gate(refused))
        }
        Err(e) => {
            warn!(
                athlete = %subject.athlete_user_id,
                group_id = %subject.group_id,
                error = %e,
                "coach's athlete thread: the coaching gates could not be read; nothing of the athlete is shown"
            );
            Err(SubjectRefusal::Unconfirmed)
        }
    }
}

/// Whether what a turn in `conv` says may be filed on its sender's own
/// account: the facts extracted from it, which compose the sender's dossier,
/// and the advice captured from it, which is evaluated against the sender's
/// own training and reinforces their playbooks.
///
/// Not in a coach's thread about one athlete. Every turn there is about the
/// athlete, in their coach's words: filed under the sender, "her FTP is 250"
/// becomes the coach's own FTP and reaches every conversation that reads the
/// coach's dossier, a group room the coach shares with other members among
/// them. Nor is it filed under the athlete: what their coach says of them is
/// not the athlete's own statement, and the athlete never agreed to it
/// entering their memory. The thread's messages are its record.
#[must_use]
pub fn files_on_sender(conv: &ConversationRecord) -> bool {
    conv.subject().is_none()
}

/// The turn's standing in a coach's thread about one athlete, or `None` for
/// every other conversation.
pub async fn resolve(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    sender: &str,
) -> Option<SubjectTurn> {
    let subject = conv.subject()?;
    let (access, coach) = match subject_access(repos, subject, sender).await {
        Ok(granted) => granted,
        Err(refused) => return Some(SubjectTurn::Refused(refused)),
    };
    let target = CoachDraftTarget::new(access.tenant, access.athlete_user_id, coach);
    let view = match coach_plan_view(repos, &target).await {
        Ok(view) => Some(view),
        // Not served over this transport: the turn goes on without the plan.
        Err(e) if e.code == ErrorCode::UnavailableOverTransport => None,
        Err(e) => {
            warn!(error = %e, "coach's athlete thread: the athlete's plan could not be read");
            None
        }
    };
    let today = athlete_today(repos, access.athlete_user_id).await;
    Some(SubjectTurn::Granted(Box::new(GrantedSubject {
        access,
        coach,
        view,
        today,
    })))
}

/// The prompt block for the turn: one line when refused, the coach's view of
/// the athlete when granted.
pub async fn block(ctx: &ChatPipelineContext, turn: &SubjectTurn) -> String {
    match turn {
        SubjectTurn::Refused(refused) => format!("{HEADING}{}", refused.line()),
        SubjectTurn::Granted(granted) => {
            let training = roster_line(ctx, granted).await;
            let wellness = wellness(ctx, granted).await;
            let shown = wellness.as_ref().map(|readings| {
                let brand = brand_name(ctx.tool_runtime.provider_registry(), &readings.provider)
                    .unwrap_or(&readings.provider);
                (brand, readings)
            });
            granted_block(granted, &training, shown)
        }
    }
}

/// The athlete's training, as the coach's roster lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RosterLine {
    /// The roster's projection of their snapshot, as JSON.
    Training(String),
    /// They connect no training source.
    NoSource,
    /// Not read: the build carries no group tools, or the read failed.
    Unread,
}

/// The athlete's roster line — the projection `get_roster_overview` lists
/// for them, from the same snapshot.
#[cfg(feature = "tools-groups")]
async fn roster_line(ctx: &ChatPipelineContext, granted: &GrantedSubject) -> RosterLine {
    use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
    use pierre_tool_runtime::implementations::group_roster::project_training;

    let athlete = granted.athlete();
    match ctx
        .repos
        .provider_connections
        .get_for_user(athlete, None)
        .await
    {
        Ok(connections) if connections.is_empty() => return RosterLine::NoSource,
        Ok(_) => {}
        Err(e) => {
            warn!(error = %e, "coach's athlete thread: the athlete's sources could not be read");
            return RosterLine::Unread;
        }
    }
    let snapshots = fetch_member_snapshots(&ctx.tool_runtime, &[athlete], granted.tenant()).await;
    let Some(snapshot) = snapshots.first() else {
        return RosterLine::Unread;
    };
    match serde_json::to_string(&project_training(snapshot)) {
        Ok(json) => RosterLine::Training(json),
        Err(e) => {
            warn!(error = %e, "coach's athlete thread: the roster line did not serialise");
            RosterLine::Unread
        }
    }
}

/// Without group tools there is no roster to project from.
#[cfg(not(feature = "tools-groups"))]
async fn roster_line(_ctx: &ChatPipelineContext, _granted: &GrantedSubject) -> RosterLine {
    RosterLine::Unread
}

/// The wellness the coach's link filed under the athlete's `wellness_read`
/// consent, or `None` without that consent (or when it could not be read).
async fn wellness(ctx: &ChatPipelineContext, granted: &GrantedSubject) -> Option<LinkedWellness> {
    match linked_wellness(
        &ctx.repos,
        ctx.tool_runtime.provider_registry().as_ref(),
        granted.coach,
        granted.athlete(),
        granted.access.group.id,
        Utc::now(),
    )
    .await
    {
        Ok(wellness) => wellness,
        Err(e) => {
            warn!(error = %e, "coach's athlete thread: the linked wellness could not be read");
            None
        }
    }
}

/// A person, group or agent name as it may enter the prompt: one line,
/// defanged, bounded.
fn name(raw: &str) -> String {
    display_line(raw, ROSTER_NAME_MAX_CHARS)
}

/// The block for a granted turn. `wellness` is the linked wellness with the
/// brand of the platform it came through.
fn granted_block(
    granted: &GrantedSubject,
    training: &RosterLine,
    wellness: Option<(&str, &LinkedWellness)>,
) -> String {
    let athlete = name(&granted.access.display_name);
    let group = name(&granted.access.group.name);
    let mut out = String::with_capacity(2_048);
    out.push_str(HEADING);
    let _ = write!(
        out,
        "The person writing is {athlete}'s coach, not {athlete}. This thread is the coach's \
         private workspace for {athlete}'s training plan, opened from the group '{group}'; \
         {athlete} does not read it.\n\n\
         - Plan and calendar tools act on {athlete} by default: leave `athlete` out. Another \
         athlete's plan is never worked on here — the coach opens that athlete's own thread \
         from Group info.\n\
         - Every plan save is a draft {athlete} cannot see. It reaches them only when the \
         coach publishes it with `publish_training_plan`, and publishing notifies {athlete}. \
         A publish that would overwrite a week {athlete} changed since it was drafted, or a \
         season {athlete} re-laid, is refused and names each such week; publish over them \
         with `overwrite_athlete_changes` only when the coach says to.\n\
         - Tools that read \"your own\" data — activities, sleep, recovery, profile, goals, \
         physiology — read the coach's own account, never {athlete}'s. Never present what \
         they return as {athlete}'s.\n\
         - Nothing said here is saved to the coach's own account: the tools that would write \
         it — facts, notes, follow-ups, commitments, goals, physiology — are not offered in \
         this thread.\n"
    );
    training_section(&mut out, &athlete, training);
    plan_section(&mut out, &athlete, granted.view.as_ref());
    if let Some((brand, wellness)) = wellness {
        wellness_section(&mut out, &athlete, brand, wellness);
    }
    out
}

fn training_section(out: &mut String, athlete: &str, training: &RosterLine) {
    match training {
        RosterLine::Training(json) => {
            let _ = write!(
                out,
                "\n### {athlete}'s training, as the coach's roster shows it\n\n{json}\n\n\
                 `form` is a share of {athlete}'s own fitness: never compare it with another \
                 athlete's, and never read it as an absolute number.\n"
            );
        }
        RosterLine::NoSource => {
            let _ = write!(
                out,
                "\n### {athlete}'s training\n\n{athlete} connects no training source yet, so \
                 there is no training to read.\n"
            );
        }
        RosterLine::Unread => {}
    }
}

fn plan_section(out: &mut String, athlete: &str, view: Option<&CoachPlanView>) {
    let _ = write!(out, "\n### {athlete}'s plan\n\n");
    let Some(view) = view else {
        let _ = writeln!(out, "{athlete}'s plan could not be read on this turn.");
        return;
    };
    match &view.live {
        Some(live) => {
            let _ = writeln!(
                out,
                "Live season: aimed at {} on {}, {} week(s) stored.",
                name(&live.plan.goal_race.name),
                live.plan.goal_race.date,
                live.weeks.len()
            );
        }
        None => {
            let _ = writeln!(out, "{athlete} has no live season yet.");
        }
    }
    let Some(draft) = &view.draft else {
        let _ = writeln!(
            out,
            "The coach holds no draft of it: the next plan save opens one.\n\
             The training plan section below is {athlete}'s live season."
        );
        return;
    };
    let drafted = if draft.drafted_weeks.is_empty() {
        "none yet (the outline only)".to_owned()
    } else {
        draft.drafted_weeks.join(", ")
    };
    let _ = writeln!(
        out,
        "The coach's draft (unpublished): aimed at {} on {}; last saved {}; weeks drafted: \
         {drafted}.",
        name(&draft.plan.goal_race.name),
        draft.plan.goal_race.date,
        draft.state.updated_at.format("%Y-%m-%d %H:%M UTC"),
    );
    let state = &draft.state;
    if state.is_clear() {
        let _ = writeln!(out, "Publishing it now overwrites nothing of {athlete}'s.");
    }
    if !state.conflicts.is_empty() {
        let weeks: Vec<&str> = state
            .conflicts
            .iter()
            .map(|conflict| conflict.week_start.as_str())
            .collect();
        let _ = writeln!(
            out,
            "Publishing it now would replace {athlete}'s own changes to the week(s) of {}: \
             {athlete} changed them after the coach drafted them.",
            weeks.join(", ")
        );
    }
    if state.season_relaid {
        let _ = writeln!(
            out,
            "{athlete}'s season was re-laid after the draft began, so the drafted weeks may no \
             longer fit it."
        );
    }
    let _ = writeln!(
        out,
        "The training plan section below is the coach's draft over {athlete}'s live weeks."
    );
}

fn wellness_section(out: &mut String, athlete: &str, brand: &str, wellness: &LinkedWellness) {
    let _ = write!(
        out,
        "\n### {athlete}'s wellness, through the coach's {brand} link\n\n"
    );
    if wellness.is_empty() {
        let _ = writeln!(
            out,
            "{athlete} lets the link sync their wellness, but it filed none in the last \
             {LINKED_WELLNESS_DAYS} days."
        );
        return;
    }
    let mut readings: Vec<String> = Vec::new();
    if let Some(hrv) = wellness.hrv_ms {
        readings.push(format!("HRV {:.0} ms ({})", hrv.value, hrv.date));
    }
    if let Some(rhr) = wellness.resting_heart_rate {
        readings.push(format!(
            "resting heart rate {} bpm ({})",
            rhr.value, rhr.date
        ));
    }
    if let Some(sleep) = wellness.sleep_seconds {
        readings.push(format!(
            "sleep {}h{:02} (night ending {})",
            sleep.value / 3_600,
            (sleep.value % 3_600) / 60,
            sleep.date
        ));
    }
    if let Some(weight) = wellness.weight_kg {
        readings.push(format!("weight {:.1} kg ({})", weight.value, weight.date));
    }
    let _ = writeln!(
        out,
        "Newest in the last {LINKED_WELLNESS_DAYS} days: {}.",
        readings.join("; ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_before_the_membership_gate_names_nobody() {
        for refusal in [
            SubjectRefusal::Gate(CoachedAthleteRefusal::NotGroupCoach),
            SubjectRefusal::Gate(CoachedAthleteRefusal::NotAMember),
            SubjectRefusal::Unconfirmed,
        ] {
            let line = refusal.line();
            assert!(
                line.starts_with("The coach can no longer act for this thread's athlete: "),
                "{line}"
            );
            assert_eq!(line.lines().count(), 1, "one line: {line}");
        }
    }

    #[test]
    fn a_revoked_consent_says_how_it_comes_back() {
        let line = SubjectRefusal::Gate(CoachedAthleteRefusal::CoachSharingRevoked {
            athlete: "Mira\n## Forged".to_owned(),
            group: "Tempo Squad".to_owned(),
        })
        .line();
        assert!(
            line.contains("Mira ## Forged has stopped sharing"),
            "{line}"
        );
        assert!(line.contains("`/group consent coach yes`"), "{line}");
        assert_eq!(line.lines().count(), 1, "a name cannot open a line: {line}");
    }
}
