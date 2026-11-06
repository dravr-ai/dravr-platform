// ABOUTME: Whose data a read tool reads: the caller's own, or a coached athlete's, resolved once at the dispatch chokepoint
// ABOUTME: The thread's athlete by default in a coach's thread, `athlete=` over MCP/A2A only, under the coaching gates every time
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Read subject
//!
//! A read tool runs as its caller. A group's human coach asks about one of
//! their athletes, though, and a tool that reads the caller's own account
//! answers with the coach's data (carnet#1387: "Tes activités (26)" were the
//! coach's own rides). Each tool declares what it reads when it registers
//! ([`DataClass`], beside its category); for a tool of the training class
//! the dispatch chokepoint resolves *whose* data it reads before the body
//! runs, here, once, rather than every tool resolving it again:
//!
//! 1. An `athlete` argument that did not arrive over an external transport is
//!    refused. Chat never offers it, so a name in a chat call is the model's
//!    guess, and a guess must not quietly read someone — least of all the
//!    coach.
//! 2. In a coach's thread about one athlete ([`TurnSeat::AthleteThread`]) the
//!    subject is the thread's athlete, with no argument: the conversation row
//!    names them ([`ConversationRecord::subject`]). A thread that names no
//!    one reads no one.
//! 3. Over MCP or A2A, a coach names a coached athlete with `athlete=`, by
//!    roster name ([`match_coached_member`]).
//! 4. Otherwise the caller reads their own data.
//!
//! Either way the subject passes [`coached_athlete_access`] on every call —
//! the coach attachment, a live membership, the athlete's
//! `coach_sharing_consent` and one home tenant — so a consent revoked
//! mid-thread refuses the next read. Each refusal is in-band, worded for the
//! model to relay.
//!
//! The plan tools' scope ([`crate::implementations::plan_scope`]) resolves
//! its own `athlete` the same way and shares the name match and the gate
//! wording kept here.

use std::fmt;
use std::sync::Arc;

use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_mcp_schema::PropertySchema;
use pierre_services::athlete_display_name::fetch_user_display_name;
use pierre_services::coached_athlete::{
    coached_athlete_access, CoachedAthleteAccess, CoachedAthleteRefusal,
};
use serde_json::json;
use uuid::Uuid;

use crate::coach_seat::TurnSeat;
use crate::implementations::training_plan_authorship::load_conversation;
use crate::protocol::types::UniversalResponse;
use crate::runtime::ToolRuntime;

/// The argument a tool takes when it can act on an athlete other than the
/// caller: a coach names a coached athlete with it. The plan and calendar
/// tools declare it themselves, and in a coach's thread about one athlete it
/// defaults to that athlete; a training read is listed with it over MCP and
/// A2A only ([`athlete_property`]).
pub(crate) const ATHLETE_ARGUMENT: &str = "athlete";

/// `error_code` of every refusal a read-subject resolution answers with.
pub(crate) const READ_SUBJECT_ERROR_CODE: &str = "read_subject_refused";

/// What a tool reads, declared when it registers, beside its category.
///
/// The class decides whose data the tool may read for a coach: a
/// [`DataClass::Training`] tool reads a coached athlete's when the call
/// resolves one (`resolve_read_subject`), and a [`DataClass::NoSubject`]
/// tool reads no person's data at all, so it answers the same whoever asks.
/// A tool that reads data and declares no class reads its caller's own, and
/// a coach's thread about one athlete withholds it
/// ([`TurnSeat::AthleteThread`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClass {
    /// The athlete's training: activities, their detail and streams, the
    /// daily load history and what is derived from it. Gated by the
    /// athlete's `coach_sharing_consent`.
    Training,
    /// Reads no person's data: route and weather lookups, food and mobility
    /// references, pure calculators, the agent catalogue.
    NoSubject,
}

/// How the call named the athlete it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubjectSource {
    /// The coach's thread about the athlete names them.
    Thread,
    /// An MCP or A2A caller named them with `athlete=`.
    ExternalArgument,
}

impl SubjectSource {
    /// The word the dispatch log names the source by.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::ExternalArgument => "external_argument",
        }
    }
}

/// A coached athlete a call reads, and the coach it reads them for.
#[derive(Debug, Clone)]
pub struct ReadSubject {
    /// The coach who asked: charged for the call, and whose turn it belongs to.
    pub(crate) requester: Uuid,
    /// The coach's own tenant.
    pub(crate) requester_tenant: TenantId,
    /// The athlete, their tenant and roster name, and the group whose gates
    /// let the coach read them, as checked for this call.
    pub(crate) access: CoachedAthleteAccess,
    /// How the call named the athlete.
    pub(crate) via: SubjectSource,
}

impl ReadSubject {
    /// The athlete whose data the call reads.
    #[must_use]
    pub(crate) const fn athlete(&self) -> Uuid {
        self.access.athlete_user_id
    }

    /// The athlete's one home tenant, where their data lives.
    #[must_use]
    pub(crate) const fn tenant(&self) -> TenantId {
        self.access.tenant
    }
}

/// What the chokepoint knows about a call before it resolves whose data the
/// call reads.
pub(crate) struct ReadSubjectRequest<'request> {
    /// The runtime the gates read through.
    pub resources: &'request Arc<dyn ToolRuntime>,
    /// The caller.
    pub requester: Uuid,
    /// The caller's own tenant, when the call carries one.
    pub requester_tenant: Option<TenantId>,
    /// The turn's seat.
    pub seat: TurnSeat,
    /// The originating conversation, for a call from a chat turn.
    pub conversation_id: Option<&'request str>,
    /// The `athlete` argument, as the call sent it.
    pub athlete: Option<&'request str>,
    /// Whether the call arrived over an external transport whose caller
    /// writes its own reply: an MCP client or an A2A agent.
    pub named_externally: bool,
    /// The tool asking, for the refusal's wording.
    pub tool_name: &'request str,
}

impl fmt::Debug for ReadSubjectRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadSubjectRequest")
            .field("requester", &self.requester)
            .field("requester_tenant", &self.requester_tenant)
            .field("seat", &self.seat)
            .field("conversation_id", &self.conversation_id)
            .field("athlete", &self.athlete)
            .field("named_externally", &self.named_externally)
            .field("tool_name", &self.tool_name)
            .finish_non_exhaustive()
    }
}

/// Resolve whose data a training-class read reads: `None` for the caller's
/// own, a [`ReadSubject`] for a coached athlete, or the in-band refusal the
/// call answers with instead, before anything is read.
///
/// LIMITATION(registre#1445): `resolve_read_subject` lets a coach read a coached athlete's
/// training under their consents and leaves the athlete no record of it: no log they can
/// open says which coach read what, and when.
///
/// # Errors
///
/// Returns the repository error from the conversation, group, member or
/// tenant lookups.
pub(crate) async fn resolve_read_subject(
    request: ReadSubjectRequest<'_>,
) -> AppResult<Result<Option<ReadSubject>, UniversalResponse>> {
    let athlete = request
        .athlete
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if athlete.is_some() && !request.named_externally {
        return Ok(Err(athlete_argument_refusal(request.tool_name)));
    }
    if request.seat == TurnSeat::AthleteThread {
        return thread_subject(&request).await;
    }
    match athlete {
        Some(name) => named_subject(&request, name).await,
        None => Ok(Ok(None)),
    }
}

/// The subject of a call made in a coach's thread about one athlete: that
/// athlete, re-checked against the coaching gates, or a refusal when the
/// thread names no one the caller coaches.
async fn thread_subject(
    request: &ReadSubjectRequest<'_>,
) -> AppResult<Result<Option<ReadSubject>, UniversalResponse>> {
    let Some(requester_tenant) = request.requester_tenant else {
        return Ok(Err(unbound_thread_refusal(request.tool_name)));
    };
    let repos = request.resources.repos();
    let conversation = load_conversation(
        repos,
        request.conversation_id,
        requester_tenant,
        &request.requester.to_string(),
    )
    .await?;
    let Some(subject) = conversation.as_ref().and_then(ConversationRecord::subject) else {
        return Ok(Err(unbound_thread_refusal(request.tool_name)));
    };
    match coached_athlete_access(
        repos,
        request.requester,
        subject.athlete_user_id,
        subject.group_id,
    )
    .await?
    {
        Ok(access) => Ok(Ok(Some(ReadSubject {
            requester: request.requester,
            requester_tenant,
            access,
            via: SubjectSource::Thread,
        }))),
        Err(refused) => {
            let message = gate_message(&refused, GateObject::Training, || {
                thread_unreached(&refused, GateObject::Training)
            });
            Ok(Err(refusal(refused.reason(), message)))
        }
    }
}

/// The subject an external caller named with `athlete=`: the one coached
/// member the name matches, once the coaching gates let the caller read them.
async fn named_subject(
    request: &ReadSubjectRequest<'_>,
    name: &str,
) -> AppResult<Result<Option<ReadSubject>, UniversalResponse>> {
    const YOURSELF: &str = "omit `athlete` to read your own data.";
    let repos = request.resources.repos();
    let no_match = || MemberMiss::NoMatch.message(name, YOURSELF);
    let Some(requester_tenant) = request.requester_tenant else {
        return Ok(Err(refusal("no_match", no_match())));
    };
    let found = match match_coached_member(repos, request.requester, name, None).await? {
        Ok(found) => found,
        Err(miss) => return Ok(Err(refusal(miss.reason(), miss.message(name, YOURSELF)))),
    };
    // Gates 2 to 4, by id. The match prefers a group where the athlete shares
    // with the coach, so a consent refusal names the most permissive row; a
    // membership that changed since the match reads as no match.
    match coached_athlete_access(repos, request.requester, found.user_id, found.group.id).await? {
        Ok(access) => Ok(Ok(Some(ReadSubject {
            requester: request.requester,
            requester_tenant,
            access,
            via: SubjectSource::ExternalArgument,
        }))),
        Err(refused) => Ok(Err(refusal(
            refused.reason(),
            gate_message(&refused, GateObject::Training, no_match),
        ))),
    }
}

/// The `athlete` property a training-class tool advertises over MCP and A2A.
///
/// One short description shared by every such tool, so the argument costs
/// each listing about a hundred bytes. Chat never offers it: in a coach's
/// thread the athlete is implicit, and anywhere else in chat a name would be
/// the model's guess.
#[must_use]
pub(crate) fn athlete_property() -> (String, PropertySchema) {
    (
        ATHLETE_ARGUMENT.to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Roster name of an athlete you coach who shares their training with you: \
                 reads theirs instead of yours."
                    .to_owned(),
            ),
            ..Default::default()
        },
    )
}

/// A refusal in the shape every read-subject resolution answers with.
fn refusal(reason: &str, message: String) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": READ_SUBJECT_ERROR_CODE,
            "reason": reason,
        })),
        error: Some(message),
        metadata: None,
    }
}

/// The refusal of an `athlete` argument a chat model sent.
fn athlete_argument_refusal(tool_name: &str) -> UniversalResponse {
    refusal(
        "athlete_argument",
        format!(
            "'{tool_name}' takes no `athlete` here. In a coach's thread about one athlete, data \
             tools read that athlete with `athlete` left out; anywhere else in this chat they \
             read only the caller's own data. To read a coached athlete's training, the coach \
             opens that athlete's thread from Group info."
        ),
    )
}

/// The refusal of a read in a coach's thread that names no athlete they
/// coach: nothing is read rather than the coach's own data.
fn unbound_thread_refusal(tool_name: &str) -> UniversalResponse {
    refusal(
        "thread_unbound",
        format!(
            "'{tool_name}' reads nothing here: this thread is not bound to an athlete you coach, \
             so there is no one whose data it could read."
        ),
    )
}

/// The refusal of a nested call, made inside a read of a coached athlete, to
/// a tool that does not read their training: it would read data the scope
/// was never resolved for.
#[must_use]
pub(crate) fn outside_class_refusal(tool_name: &str) -> UniversalResponse {
    refusal(
        "outside_subject_class",
        format!(
            "'{tool_name}' does not read an athlete's training, so it cannot run inside a read \
             of a coached athlete."
        ),
    )
}

/// The refusal of `argument`, which would make `tool_name` read the coached
/// athlete's wellness — sleep, HRV, recovery — on a read of their training:
/// their training is what the coach reads here.
#[cfg(feature = "tools-analytics")]
#[must_use]
pub(crate) fn wellness_refusal(tool_name: &str, argument: &str) -> UniversalResponse {
    refusal(
        "wellness_not_read",
        format!(
            "'{tool_name}' with `{argument}` reads the athlete's sleep and recovery, which a coach \
             does not read here — only their training. Leave `{argument}` out to read their \
             training load alone."
        ),
    )
}

/// What a coach's thread says when the gates fail before the athlete is
/// shown to be theirs: those two refusals name nobody.
pub(crate) fn thread_unreached(refused: &CoachedAthleteRefusal, object: GateObject) -> String {
    let what = object.their();
    match refused {
        CoachedAthleteRefusal::NotGroupCoach => format!(
            "This thread works on an athlete of a group you no longer coach, so {what} is out of \
             reach here."
        ),
        _ => format!(
            "This thread's athlete is no longer a member of the group you coach them in, so \
             {what} is out of reach here."
        ),
    }
}

/// What a coaching gate refusal is about: the athlete's plan, or their
/// training.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateObject {
    /// Their season plan and calendar.
    Plan,
    /// Their training data.
    Training,
}

impl GateObject {
    /// The object, as a refusal names it.
    const fn their(self) -> &'static str {
        match self {
            Self::Plan => "their plan",
            Self::Training => "their training",
        }
    }
}

/// The words of a coaching gate refusal about `object`. `unreached` words the
/// two that fail before the athlete is shown to be the requester's — those
/// name nobody.
pub(crate) fn gate_message(
    refused: &CoachedAthleteRefusal,
    object: GateObject,
    unreached: impl FnOnce() -> String,
) -> String {
    match (refused, object) {
        (CoachedAthleteRefusal::NotGroupCoach | CoachedAthleteRefusal::NotAMember, _) => {
            unreached()
        }
        (CoachedAthleteRefusal::CoachSharingRevoked { athlete, group }, _) => format!(
            "{athlete} has stopped sharing their data with you, the coach of '{group}'. They can \
             share it again with `/group consent coach yes`."
        ),
        (CoachedAthleteRefusal::NoTenant { athlete }, GateObject::Plan) => {
            format!("{athlete} belongs to no tenant, so there is nowhere to keep their plan.")
        }
        (CoachedAthleteRefusal::NoTenant { athlete }, GateObject::Training) => {
            format!("{athlete} belongs to no tenant, so there is no training of theirs to read.")
        }
        (CoachedAthleteRefusal::SeveralTenants { athlete }, GateObject::Plan) => format!(
            "{athlete} belongs to several tenants — acting on their plan is not supported, \
             because the plan would be visible on one of their surfaces and missing on another."
        ),
        (CoachedAthleteRefusal::SeveralTenants { athlete }, GateObject::Training) => format!(
            "{athlete} belongs to several tenants — reading their training is not supported, \
             because it would read one tenant's data and miss the other's."
        ),
    }
}

/// The one coached member a name query matched, with the group row that
/// authorizes acting for them.
#[derive(Debug)]
pub struct CoachedMatch {
    /// A group the requester coaches the member in: one where the member
    /// shares with them when any does, so a refusal names the most permissive
    /// row; else the first.
    pub group: CoachingGroup,
    /// The member.
    pub user_id: Uuid,
    /// Their display name ([`fetch_user_display_name`]).
    pub(crate) display_name: String,
    /// Whether the member shares with the requester in `group`.
    pub(crate) consented: bool,
}

/// Why a name query matched no one coached member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberMiss {
    /// The query matched only the requester.
    Requester,
    /// No live member of a group the requester coaches matched.
    NoMatch,
    /// Several distinct members matched: their display names, sorted.
    Ambiguous(Vec<String>),
}

impl MemberMiss {
    /// What the model relays for a query `query` that resolved to no one
    /// member; `yourself` says what to do when it named the requester.
    pub(crate) fn message(&self, query: &str, yourself: &str) -> String {
        match self {
            Self::Requester => format!("'{query}' is you — {yourself}"),
            Self::NoMatch => format!(
                "No athlete matching '{query}' in a group you coach — the athlete must be a \
                 member of a group you are attached to as its coach (a coach invite), not merely \
                 a group you belong to."
            ),
            Self::Ambiguous(names) => format!(
                "'{query}' is ambiguous — several athletes you coach match: {}. Use a more \
                 specific name.",
                names.join(", ")
            ),
        }
    }

    /// The reason a refusal of this miss carries.
    pub(crate) const fn reason(&self) -> &'static str {
        match self {
            Self::Requester => "self",
            Self::NoMatch => "no_match",
            Self::Ambiguous(_) => "ambiguous",
        }
    }
}

/// The one live member of a group `requester` is the human coach of whose
/// display name contains `query`, case aside.
///
/// Walked from the groups the requester is attached to as their coach, never
/// from the groups they merely belong to; `within` narrows the walk to one of
/// them — the room a reply would be read in. The same athlete in two coached
/// groups is one person; the requester is never their own athlete.
///
/// The outer `Err` is a repository failure; the inner `Err` says why no one
/// member matched.
///
/// # Errors
///
/// Returns the repository error from the group or member lookups.
pub async fn match_coached_member(
    repos: &RepositoryRegistry,
    requester: Uuid,
    query: &str,
    within: Option<Uuid>,
) -> AppResult<Result<CoachedMatch, MemberMiss>> {
    let query_lower = query.to_lowercase();
    let mut matches: Vec<CoachedMatch> = Vec::new();
    let mut requester_matched = false;
    for group in repos.groups.list_groups_coached_by(requester).await? {
        if !group.is_active || within.is_some_and(|pin| pin != group.id) {
            continue;
        }
        for member in repos.groups.list_members(&group.id.to_string()).await? {
            if member.left_at.is_some() {
                continue;
            }
            let display_name = fetch_user_display_name(repos, member.user_id).await;
            if !display_name.to_lowercase().contains(&query_lower) {
                continue;
            }
            if member.user_id == requester {
                requester_matched = true;
                continue;
            }
            matches.push(CoachedMatch {
                group: group.clone(),
                user_id: member.user_id,
                display_name,
                consented: member.coach_sharing_consent,
            });
        }
    }

    let Some(first) = matches.first() else {
        return Ok(Err(if requester_matched {
            MemberMiss::Requester
        } else {
            MemberMiss::NoMatch
        }));
    };
    // Ambiguous only when DISTINCT athletes match — the same athlete in two
    // coached groups is one person.
    if matches.iter().any(|m| m.user_id != first.user_id) {
        let mut names: Vec<String> = matches.iter().map(|m| m.display_name.clone()).collect();
        names.sort_unstable();
        names.dedup();
        return Ok(Err(MemberMiss::Ambiguous(names)));
    }
    let at = matches.iter().position(|m| m.consented).unwrap_or(0);
    Ok(Ok(matches.swap_remove(at)))
}

/// Whom a name that does not name a thread's athlete names instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OtherName {
    /// Another athlete the requester coaches, by roster name.
    Athlete(String),
    /// The requester themself.
    Requester,
    /// No one the requester coaches, and not them.
    Nobody,
}

/// Whether `query` names the athlete whose roster name is `display_name`: in
/// the thread about them, a shared first name means them.
pub(crate) fn names_subject(display_name: &str, query: &str) -> bool {
    display_name.to_lowercase().contains(&query.to_lowercase())
}

/// Whom `query` names when it does not name a thread's athlete: another
/// athlete `requester` coaches, the requester, or no one.
///
/// # Errors
///
/// Returns the repository error from the group or member lookups.
pub(crate) async fn other_name(
    repos: &RepositoryRegistry,
    requester: Uuid,
    query: &str,
) -> AppResult<OtherName> {
    Ok(
        match match_coached_member(repos, requester, query, None).await? {
            Ok(other) => OtherName::Athlete(other.display_name),
            Err(MemberMiss::Requester) => OtherName::Requester,
            // A coach attached to a group holds no membership in it, so their
            // own name is asked of their own row.
            Err(MemberMiss::NoMatch | MemberMiss::Ambiguous(_)) => {
                if names_subject(&fetch_user_display_name(repos, requester).await, query) {
                    OtherName::Requester
                } else {
                    OtherName::Nobody
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_athlete_argument_refusal_points_the_coach_at_the_athletes_thread() {
        let refused = athlete_argument_refusal("get_activities");
        assert!(!refused.success, "an athlete argument from chat is refused");
        assert_eq!(
            refused.result.as_ref().unwrap()["reason"],
            "athlete_argument",
            "the refusal carries the reason a test branches on"
        );
        let message = refused.error.unwrap();
        assert!(
            message.starts_with("'get_activities' takes no `athlete` here"),
            "bug: the refusal names the tool and the argument it refused: {message}"
        );
        assert!(
            message.contains("thread from Group info"),
            "bug: the refusal points the coach at the athlete's thread: {message}"
        );
    }

    #[test]
    fn a_revoked_consent_names_the_athlete_and_how_they_share_again_for_either_object() {
        let refused = CoachedAthleteRefusal::CoachSharingRevoked {
            athlete: "Mira".to_owned(),
            group: "Tempo".to_owned(),
        };
        for object in [GateObject::Plan, GateObject::Training] {
            let message = gate_message(&refused, object, || unreachable!("named"));
            assert!(
                message.contains("Mira has stopped sharing") && message.contains("'Tempo'"),
                "the consent refusal names the athlete and the group: {message}"
            );
        }
    }

    #[test]
    fn the_tenant_refusals_word_what_was_asked_for() {
        let refused = CoachedAthleteRefusal::NoTenant {
            athlete: "Mira".to_owned(),
        };
        assert!(
            gate_message(&refused, GateObject::Plan, String::new).contains("keep their plan"),
            "a plan refusal speaks of the plan"
        );
        assert!(
            gate_message(&refused, GateObject::Training, String::new)
                .contains("no training of theirs to read"),
            "a read refusal speaks of the training"
        );
    }

    #[test]
    fn a_gate_that_fails_before_the_athlete_is_reached_names_nobody() {
        let message = gate_message(
            &CoachedAthleteRefusal::NotGroupCoach,
            GateObject::Training,
            || thread_unreached(&CoachedAthleteRefusal::NotGroupCoach, GateObject::Training),
        );
        assert!(
            message.contains("no longer coach") && message.contains("their training"),
            "bug: an unreached gate says why, about the training, naming no one: {message}"
        );
    }

    #[test]
    fn a_shared_first_name_names_the_threads_athlete() {
        assert!(
            names_subject("Mira Subject", "mira"),
            "bug: a first name, case aside, names the thread's athlete"
        );
        assert!(
            !names_subject("Mira Subject", "Cole"),
            "bug: another name never names the thread's athlete"
        );
    }

    #[test]
    fn the_athlete_property_is_a_short_string() {
        let (name, schema) = athlete_property();
        assert_eq!(name, ATHLETE_ARGUMENT, "the property is the argument read");
        assert_eq!(
            schema.property_type, "string",
            "bug: a roster name is a string"
        );
        let description = schema.description.unwrap();
        assert!(
            description.len() <= 110,
            "every training tool's listing carries it: {} bytes",
            description.len()
        );
    }
}
