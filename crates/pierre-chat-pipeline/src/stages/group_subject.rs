// ABOUTME: Who a turn is about — the sender, a group they coach, or the one athlete their thread is bound to
// ABOUTME: Named once per turn; the seat decides whose data the turn reads and whether it is filed on the sender
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Who a turn is about.
//!
//! Every athlete-data stage keys on the sender, which is right in a 1:1 thread
//! and in a group the sender trains in. It is wrong for the group's coach: a
//! coach who finished onboarding owns a group whose athlete invite is still
//! unredeemed, is its only member, and an `@agent` mention there answered from
//! the coach's own CTL, rides and goals as though they were the group's
//! (carnet#741). It is wrong in a coach's thread bound to one athlete too
//! (carnet#1070), where every turn is about that athlete.
//!
//! [`turn_standing`] names the turn's seat once, right after the conversation
//! resolves, and every stage that reads, fetches or files the sender's own
//! data follows it. Prompt assembly, the tools, the activity prefetches,
//! capability recovery and advice capture ask [`TurnSeat::sender_is_subject`];
//! fact extraction asks [`TurnStanding::files_on_sender`], which on a group's
//! coach seat still files a guided walk the coach runs on themselves
//! (carnet#1084).
//!
//! The coach's seat is a role in THIS group, never a property of the account.
//! A coach-only account invited into someone else's group as an athlete is an
//! athlete there, and its own data reaches its turns as usual.

use pierre_core::models::groups::GroupRole;
use pierre_core::models::{ConversationSubject, OnboardingState, TenantId};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::ConversationRecord;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_services::coach_roster::coaches_group;
use pierre_services::coached_athlete::coached_athlete_access;
use pierre_services::intake::athlete_steps_waived;
use pierre_tool_runtime::chat_catalogue::TurnReaders;
use pierre_tool_runtime::coach_seat::{self, TurnSeat};
use tracing::warn;
use uuid::Uuid;

use crate::TurnInput;

/// Who a turn is about, named once from its conversation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TurnStanding {
    /// Whose data the turn runs against: the sender's own only on
    /// [`TurnSeat::Subject`].
    pub seat: TurnSeat,
    /// The seat the turn's tools are offered and dispatched on: [`Self::seat`],
    /// save in a coach's thread about an athlete whose coach access or
    /// consent is gone at turn start, where it is the coach's private seat
    /// (carnet#1511 call 30). The prompt still carries nothing of the coach's
    /// own context or of the athlete there, only why the thread is refused.
    pub tools: TurnSeat,
    /// The sender's seat as the coach of the conversation's group, when they
    /// hold it — what prompt assembly tells the agent about the room.
    pub coach: Option<CoachSeat>,
    /// Who reads what the turn writes: the room a call naming an athlete is
    /// decided in, which the turn's tool offer reads so it offers no
    /// `athlete` a call would be refused.
    pub readers: TurnReaders,
    /// The conversation carries a live guided walk whose own state names the
    /// sender as its subject: a walk they started on themselves.
    walk_bound_to_sender: bool,
}

impl TurnStanding {
    /// Whether what the turn says may be filed on its sender's own account,
    /// as the facts extracted from it.
    ///
    /// Always on the subject's seat, and never in a coach's thread about one
    /// athlete, where every turn is about the athlete. On a group's coach seat
    /// only the answers to a guided walk bound to the coach are filed: the
    /// walk interviews the coach about themselves, so its answers are the
    /// coach's own facts, filed on the coach as every walk files them.
    /// Anything else the coach says in the room is about the room
    /// (carnet#1084).
    #[must_use]
    pub(crate) const fn files_on_sender(self) -> bool {
        match self.seat {
            TurnSeat::Subject | TurnSeat::UnreachedThread => true,
            TurnSeat::Coach => self.walk_bound_to_sender,
            TurnSeat::AthleteThread => false,
        }
    }

    /// Whether the turn answers a guided walk bound to its sender: an
    /// interview about the sender alone, which reads no other member's card
    /// (carnet#1511 design call 4).
    #[must_use]
    pub(crate) const fn walks_the_sender(self) -> bool {
        self.walk_bound_to_sender
    }

    /// Whether what the turn said may be filed on its sender, once it ran.
    ///
    /// [`Self::files_on_sender`], and never on a turn that read another
    /// member's data: a coach's read naming an athlete (`athlete=`), a
    /// group member's activities, a quoted room message. That turn is about
    /// the member, exactly as every turn in a coach's athlete thread is
    /// (carnet#1070): filed on the sender, "her FTP is 250" would become the
    /// sender's own fact, rendered into every later turn that reads their
    /// dossier, a room's included (carnet#1422). Read from the turn's own
    /// provenance, which every member read marks, and so does a history that
    /// replays a member's rows: a follow-up restating an earlier read is
    /// about that member too.
    ///
    /// A member read the room may already read counts too: room-visible data
    /// relaxes only own-account writes, never fact extraction (carnet#1511
    /// call 12, narrowed by JF 7106050048).
    #[must_use]
    pub(crate) fn files_on_sender_after_reads(self, sender: &str) -> bool {
        self.files_on_sender() && !member_reads(sender).any
    }
}

/// What the running turn read of members other than `sender`
/// ([`coach_seat::MemberReads`]). A sender whose id does not parse is taken
/// to have read one, so nothing is filed or written on them.
pub(crate) fn member_reads(sender: &str) -> coach_seat::MemberReads {
    Uuid::parse_str(sender).map_or(coach_seat::MemberReads::every_read(), |sender| {
        coach_seat::MemberReads::of(ai_scope::current_provenance().as_ref(), sender)
    })
}

/// The turn's standing in `conv`: the coach's seat of its group, a coach's
/// thread bound to one athlete, or the sender as the turn's own subject.
///
/// A thread bound to an athlete carries no group, so it never holds the
/// coach's seat as well. The walk binding is read from the walk state the
/// turn starts with, which every guided turn — a probe's answer and the
/// walk's closing one alike — still carries.
pub(crate) async fn turn_standing(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    input: &TurnInput,
) -> TurnStanding {
    let coach = coach_seat(repos, conv, input.conversation_tenant_id, &input.user_id).await;
    let (seat, tools) = match (coach, conv.subject()) {
        (Some(_), _) => (TurnSeat::Coach, TurnSeat::Coach),
        (None, Some(subject)) => {
            let acts_for_athlete = coaches_thread_athlete(repos, subject, &input.user_id).await;
            (
                TurnSeat::AthleteThread,
                if acts_for_athlete {
                    TurnSeat::AthleteThread
                } else {
                    TurnSeat::UnreachedThread
                },
            )
        }
        (None, None) => (TurnSeat::Subject, TurnSeat::Subject),
    };
    let walk_bound_to_sender = OnboardingState::from_column(conv.onboarding_state.as_deref())
        .and_then(|walk| walk.subject_user_id)
        .is_some_and(|subject| subject == input.user_id);
    TurnStanding {
        seat,
        tools,
        coach,
        readers: match parse_uuid(&input.user_id) {
            Ok(sender) => TurnReaders::of_turn(repos, conv, input.is_direct_message, sender).await,
            Err(_) => TurnReaders::Others,
        },
        walk_bound_to_sender,
    }
}

/// Whether `sender` may act for the athlete of the thread `subject` names
/// on this turn: they hold live coach access to them and the athlete still
/// shares with the group's coach ([`coached_athlete_access`]).
///
/// Without both, the turn's tools run on the coach's private seat, not the
/// athlete's (carnet#1511 call 30, [`TurnSeat::UnreachedThread`]): no tool
/// bound to the thread's athlete is offered, since every call would refuse, and the subject-thread stage still tells the coach why. A sender
/// whose id does not parse, or gates that cannot be read, act for no one.
async fn coaches_thread_athlete(
    repos: &RepositoryRegistry,
    subject: ConversationSubject,
    sender: &str,
) -> bool {
    let Ok(sender) = parse_uuid(sender) else {
        return false;
    };
    match coached_athlete_access(repos, sender, subject.athlete_user_id, subject.group_id).await {
        Ok(access) => access.is_ok(),
        Err(e) => {
            warn!(
                athlete = %subject.athlete_user_id,
                error = %e,
                "coach gates for an athlete thread could not be read; holding the coach's private seat"
            );
            false
        }
    }
}

/// The sender holds the coach's seat of the conversation's group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CoachSeat {
    /// Members other than the coach, i.e. athletes who have joined. Zero while
    /// every invite is still unredeemed.
    pub athletes: usize,
}

impl CoachSeat {
    /// The prompt block that tells the agent who it is talking to.
    pub(crate) fn directive(self) -> String {
        let roster = if self.athletes == 0 {
            "No athlete has joined this group yet: the invite is still pending, so there is \
             no athlete data to read. Say so plainly, and help the coach with what needs none \
             — how the group works, what you will do once athletes join, how they want to \
             approach the season."
                .to_owned()
        } else {
            format!(
                "Athletes in this group: {}. Answer about them from the group context above, \
                 and from data fetched for an athlete the coach names.",
                self.athletes
            )
        };
        format!(
            "\n\n## Who you are talking to\n\n\
             The person writing is this group's coach, not an athlete in it. Their own \
             training, goals and profile are not the group's data: never present their \
             numbers as an athlete's, and never coach them on their own training here. If \
             they ask about their own training, tell them to ask in their own conversation \
             with Dravr, or in a group where they train as an athlete. To read an athlete's \
             data, call get_group_member_activities with that athlete's name. {roster}"
        )
    }
}

/// Whether `user_id` holds the coach's seat of `conv`'s group.
///
/// The seat is the group's recorded human coach while they hold coach access
/// (the one coach rule, [`coaches_group`]: a coach whose access was taken
/// back is a member here), or its owner when the owner is a coach: one
/// holding `manages_roster`, or one who said at onboarding that they coach
/// and do not train. A plain athlete who created a group with
/// friends owns it too, and stays its subject.
///
/// Every lookup failure degrades to `None` — the sender stays the subject,
/// which is what every turn did before this stage existed.
async fn coach_seat(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    tenant_id: TenantId,
    user_id: &str,
) -> Option<CoachSeat> {
    let group_id = conv.group_id.as_deref()?;
    let sender = parse_uuid(user_id).ok()?;
    let group = match repos.groups.get_group(group_id, tenant_id).await {
        Ok(group) => group?,
        Err(e) => {
            warn!(group_id, error = %e, "group lookup failed; sender stays the turn's subject");
            return None;
        }
    };
    let members = match repos.groups.list_members(group_id).await {
        Ok(members) => members,
        Err(e) => {
            warn!(group_id, error = %e, "member lookup failed; sender stays the turn's subject");
            return None;
        }
    };
    let seat = CoachSeat {
        athletes: members
            .iter()
            .filter(|m| m.user_id != sender && Some(m.user_id) != group.coach_user_id)
            .count(),
    };

    if group.coach_user_id == Some(sender) {
        return holds_coach_access(repos, sender, group_id)
            .await
            .then_some(seat);
    }
    let owns = members
        .iter()
        .any(|m| m.user_id == sender && m.role == GroupRole::Owner);
    (owns && is_coach(repos, sender, user_id).await).then_some(seat)
}

/// Whether the group's recorded coach `sender` still coaches it by the one
/// coach rule, as the coach tools' gates and the listing's room class read
/// it: a coach whose coach access was taken back reads the room as any
/// member does, never from the coach's seat. A failed lookup answers `false`.
async fn holds_coach_access(repos: &RepositoryRegistry, sender: Uuid, group_id: &str) -> bool {
    match coaches_group(repos, sender, group_id).await {
        Ok(holds) => holds,
        Err(e) => {
            warn!(group_id, error = %e, "coach access lookup failed; sender stays the turn's subject");
            false
        }
    }
}

/// Whether the account coaches: the admin grant, or the coach-only answer
/// recorded at onboarding.
async fn is_coach(repos: &RepositoryRegistry, sender: Uuid, user_id: &str) -> bool {
    match repos.users.get_global(sender).await {
        Ok(Some(user)) if user.manages_roster => return true,
        Ok(_) => {}
        Err(e) => warn!(error = %e, "user lookup failed; roster grant treated as absent"),
    }
    match repos.user_onboarding.get_onboarding_steps(user_id).await {
        Ok(steps) => athlete_steps_waived(&steps),
        Err(e) => {
            warn!(error = %e, "onboarding step lookup failed; owner treated as an athlete");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_group_says_the_invite_is_pending() {
        let directive = CoachSeat { athletes: 0 }.directive();
        assert!(directive.contains("this group's coach"));
        assert!(directive.contains("invite is still pending"));
    }

    #[test]
    fn a_joined_group_counts_its_athletes() {
        let directive = CoachSeat { athletes: 3 }.directive();
        assert!(directive.contains("Athletes in this group: 3."));
        assert!(!directive.contains("pending"));
    }

    fn standing(seat: TurnSeat, walk_bound_to_sender: bool) -> TurnStanding {
        TurnStanding {
            seat,
            tools: seat,
            coach: None,
            readers: TurnReaders::CallerAlone,
            walk_bound_to_sender,
        }
    }

    #[test]
    fn a_coach_seat_files_only_the_walk_the_coach_runs_on_themselves() {
        assert!(standing(TurnSeat::Coach, true).files_on_sender());
        assert!(!standing(TurnSeat::Coach, false).files_on_sender());
    }

    #[test]
    fn an_athlete_thread_files_nothing_and_the_subject_everything() {
        for walk in [false, true] {
            assert!(!standing(TurnSeat::AthleteThread, walk).files_on_sender());
            assert!(standing(TurnSeat::Subject, walk).files_on_sender());
        }
    }
}
