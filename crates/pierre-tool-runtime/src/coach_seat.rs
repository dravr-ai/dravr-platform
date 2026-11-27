// ABOUTME: Which tools a coach may call where they coach — their group, or their thread about one athlete
// ABOUTME: One predicate every tool surface shares: the coach's own data tools in a group, their own reads and writes in a thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's seats, as the tool layer sees them.
//!
//! Every chat-callable tool runs as the caller. Two turns are not about the
//! caller, though, and there a tool the model calls on the caller's own
//! account reads or files the wrong person's data:
//!
//! - **A group the sender coaches** (carnet#741, carnet#742): the subject is
//!   the group's athletes, so a `get_activities` with no athlete named would
//!   read the coach's own rides. A tool that reads or writes data is withheld
//!   unless its category reaches the group's athletes or the coach's
//!   catalogue.
//! - **A coach's thread about one athlete** (carnet#1070, carnet#1387): every
//!   turn there is about that athlete. A `remember_fact` would file "her FTP
//!   is 250" as the coach's own FTP, and a note, follow-up, commitment, goal
//!   or physiology write would land in the coach's dossier and reach every
//!   other conversation that reads it; a `recall_user_memory` would answer a
//!   question about her with the coach's own memory. A tool that writes data
//!   is withheld unless it takes the `athlete` argument, which there defaults
//!   to the thread's athlete. A tool that reads data is withheld unless it
//!   takes that argument or declares a data class
//!   ([`DataClass`](crate::read_subject::DataClass)): a training read reads
//!   the thread's athlete, and a wellness read too under their wellness grant
//!   ([`crate::read_subject`]); a tool that reads no person's data answers the
//!   same whoever asks. The `groups` tools take
//!   neither — in a thread their room resolves as private, and each would
//!   scan every group the coach coaches — so they stay in the room only.
//!
//! Each rule is a property of the tool, not a list of names: a self-data tool
//! added later is withheld without anyone remembering to list it.
//!
//! Advertisement and execution ask the same question, the way
//! [`crate::implementations::guided_flow`] does for guided flows: the declared
//! tool list leaves these tools out, and the executor refuses them if the
//! model calls one anyway.

use dravr_tronc::mcp::canonical::to_canonical_value;
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_llm::FunctionDeclaration;
use pierre_mcp_schema::ToolSchema;
use pierre_providers::ai_scope::Provenance;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::protocol::types::UniversalResponse;
use crate::read_subject::{self, ATHLETE_ARGUMENT, WELLNESS_GRANT_WHERE};
use crate::registry::ToolRegistry;
use crate::tool_data::WriteKind;

/// Whose data a turn's tools run against.
///
/// An enum rather than a flag because each value names a seat: the subject is
/// reading their own data, the coach is reading the group's athletes', or
/// working on one athlete's plan in their thread about them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TurnSeat {
    /// The caller is the turn's subject: every tool runs on their own data.
    #[default]
    Subject,
    /// The caller coaches the turn's group: their own data tools are withheld.
    Coach,
    /// The caller is a group's coach in their thread about one athlete of it:
    /// a tool that reads or writes data acts on that athlete or is withheld,
    /// so the coach's own data never answers a question about them and
    /// nothing said about them is filed on the coach's own account.
    AthleteThread,
    /// The caller's thread about one athlete they no longer reach: their
    /// coach access or the athlete's sharing is gone. It is the caller's
    /// private seat (carnet#1511 call 30), save the tools that act on the
    /// thread's athlete (those taking `athlete`): the thread binds them to
    /// that athlete, so every call to one is refused, and none is offered.
    UnreachedThread,
}

impl TurnSeat {
    /// Whether the caller is the turn's subject — the one seat on which their
    /// own data is what the turn is about.
    ///
    /// Off it, the caller is a coach (of the room, or in their thread about one
    /// athlete): none of their activities is read for the model on the
    /// platform's initiative, and what the turn says is not filed as theirs —
    /// save the answers to a guided walk they run on themselves in their room,
    /// which the chat pipeline's turn standing lets through.
    #[must_use]
    pub const fn sender_is_subject(self) -> bool {
        matches!(self, Self::Subject | Self::UnreachedThread)
    }

    /// Whether `tool_name` is withheld on this seat.
    ///
    /// An unknown tool is not withheld here: the executor already refuses a
    /// name the registry does not resolve.
    #[must_use]
    pub fn withholds(self, registry: &ToolRegistry, tool_name: &str) -> bool {
        match self {
            Self::Subject => false,
            Self::Coach => is_withheld_on_coach_seat(registry, tool_name),
            Self::AthleteThread => {
                writes_callers_own_data(registry, tool_name)
                    || reads_callers_own_data(registry, tool_name)
            }
            Self::UnreachedThread => acts_on_the_threads_athlete(registry, tool_name),
        }
    }

    /// The in-band refusal for `tool_name` when this seat withholds it: an
    /// ordinary refusal the model adapts to, not a security event.
    #[must_use]
    pub(crate) fn refusal(
        self,
        registry: &ToolRegistry,
        tool_name: &str,
    ) -> Option<UniversalResponse> {
        if !self.withholds(registry, tool_name) {
            return None;
        }
        let (error_code, reason, message) = match self {
            Self::Subject => return None,
            Self::Coach => (
                COACH_SEAT_ERROR_CODE,
                "coach_seat",
                coach_seat_message(tool_name),
            ),
            Self::AthleteThread if writes_callers_own_data(registry, tool_name) => (
                ATHLETE_THREAD_ERROR_CODE,
                "athlete_thread",
                athlete_thread_message(tool_name),
            ),
            Self::AthleteThread => (
                ATHLETE_THREAD_ERROR_CODE,
                "athlete_thread_read",
                athlete_thread_read_message(tool_name),
            ),
            Self::UnreachedThread => (
                ATHLETE_THREAD_ERROR_CODE,
                "thread_unreached",
                unreached_thread_message(tool_name),
            ),
        };
        Some(UniversalResponse {
            success: false,
            result: Some(json!({
                "error_code": error_code,
                "reason": reason,
            })),
            error: Some(message),
            metadata: None,
        })
    }
}

/// Categories whose data tools stay callable on the coach's seat: `groups`
/// reaches a named athlete through their own consent gate, and `store` is the
/// coach's agent catalogue, not anyone's training.
const COACH_SEAT_CATEGORIES: &[&str] = &["groups", "store"];

/// `error_code` of the refusal a tool withheld on the coach's seat returns.
pub const COACH_SEAT_ERROR_CODE: &str = "coach_seat_withheld";

/// `error_code` of the refusal a tool withheld in a coach's thread about one
/// athlete returns.
pub const ATHLETE_THREAD_ERROR_CODE: &str = "athlete_thread_withheld";

/// Whether `tool_name` is withheld while the sender holds the coach's seat.
///
/// The plan tools are among them: an athlete's plan is never posted in the
/// group. `get_group_member_plan_status`, a `groups` tool, stays callable and
/// says whether the athlete has a published plan and where their coach reads
/// it (carnet#1244).
fn is_withheld_on_coach_seat(registry: &ToolRegistry, tool_name: &str) -> bool {
    let Some(tool) = registry.get(tool_name) else {
        return false;
    };
    let touches_data = tool
        .capabilities()
        .intersects(ToolCapabilities::READS_DATA | ToolCapabilities::WRITES_DATA);
    touches_data
        && !registry
            .category_for_tool(tool_name)
            .is_some_and(|category| COACH_SEAT_CATEGORIES.contains(&category))
}

/// Whether `tool_name` writes on the caller's own account: it writes data and
/// takes no [`ATHLETE_ARGUMENT`] to act on anyone else. In a coach's thread
/// about one athlete a tool that takes the argument acts on that athlete when
/// it is left out, so only these write the coach's own account there.
fn writes_callers_own_data(registry: &ToolRegistry, tool_name: &str) -> bool {
    writes_on_caller(registry, tool_name, true).is_some()
}

/// Whether `tool_name` stores free text a later prompt reads, wherever it
/// writes: its declared kind, and free text for a write that declared none,
/// the stricter kind ([`writes_on_caller`]). What the grant taint of `/mcp`
/// and A2A refuses (carnet#1511 call 30).
pub(crate) fn files_free_text(registry: &ToolRegistry, tool_name: &str) -> bool {
    writes_on_caller(registry, tool_name, false) == Some(WriteKind::FreeText)
}

/// What a call to `tool_name` writes on the caller's own account, `None`
/// when it writes nothing there: it writes data, and acts on no one else,
/// because it takes no [`ATHLETE_ARGUMENT`] or `names_athlete` is false.
///
/// The kind is the one the tool declared ([`ToolRegistry::write_kind`],
/// carnet#1511 call 16). A write that declared none is taken as free text, the
/// stricter kind: a tool registered without its declaration is refused, never
/// let through.
fn writes_on_caller(
    registry: &ToolRegistry,
    tool_name: &str,
    names_athlete: bool,
) -> Option<WriteKind> {
    let tool = registry.get(tool_name)?;
    (tool.capabilities().contains(ToolCapabilities::WRITES_DATA)
        && !(names_athlete && takes_athlete_argument(&tool.definition().input_schema)))
    .then(|| {
        registry
            .write_kind(tool_name)
            .unwrap_or(WriteKind::FreeText)
    })
}

/// Whether the turn `provenance` accumulates read another member's data
/// wider than the room it runs in already reads: what refuses a write to the
/// caller's own account (carnet#1511 call 12, as narrowed).
///
/// Any member other than `caller`: a coach's read naming an athlete, a
/// roster, a group member's activities, a quoted room message, or a replayed
/// history about one (carnet#1422), unless every reader of the turn's room
/// may already read them.
#[must_use]
pub(crate) fn read_another_member(provenance: Option<&Provenance>, caller: Uuid) -> bool {
    provenance.is_some_and(|provenance| {
        provenance
            .members_read_beyond_room()
            .iter()
            .any(|member| *member != caller)
    })
}

/// Whether the topic drew on a member other than `caller` and `target`.
///
/// What refuses a write of free text onto `target`'s account (carnet#1511
/// call 24). A coach's note or session written for Alex from Bea's data
/// would reach Alex's calendar and plan, read by Alex with no consent of
/// Bea's. Every read the topic carries since its newest divider counts, the
/// turns before this one replayed included: "roster, then prescribe for
/// Alex" in one topic is refused, and points at Alex's thread. A read of
/// Alex herself is the write's own subject, and a write onto her draws on no
/// one (call 26). A replayed legacy row whose reads no row names may carry
/// anyone, so it draws on another than every target (carnet#1511 A8).
#[must_use]
pub(crate) fn drew_on_others_than(
    provenance: Option<&Provenance>,
    caller: Uuid,
    target: Uuid,
) -> bool {
    provenance.is_some_and(Provenance::drew_on_unknown_member)
        || drew_on_named_others_than(provenance, caller, target)
}

/// Whether the topic drew on a member other than `caller` and `target`
/// that a read names: [`drew_on_others_than`] without the legacy rows no
/// read names.
fn drew_on_named_others_than(provenance: Option<&Provenance>, caller: Uuid, target: Uuid) -> bool {
    provenance.is_some_and(|provenance| {
        provenance
            .members_drawn_on()
            .iter()
            .any(|member| *member != caller && *member != target)
    })
}

/// What a turn read of other members, as the write refusals ask it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemberReads {
    /// It read another member wider than its room's own view
    /// ([`read_another_member`]).
    pub beyond_room: bool,
    /// It drew on another member at all, the room's own view included
    /// ([`Provenance::members_drawn_on`] names one other than the caller):
    /// what refuses fact extraction and every write that stores free text
    /// a later prompt reads ([`WriteKind::FreeText`]), since room-visible
    /// data relaxes only own-account writes (carnet#1511 calls 16 and 24).
    pub any: bool,
    /// It drew on two or more members other than the caller, on one the
    /// caller did not read as their coach, or on members no row names (a
    /// replayed legacy row, carnet#1511 A8): a free-text write onto any
    /// athlete the caller may name then draws on another, and none is taken
    /// ([`drew_on_others_than`], carnet#1511 call 24).
    pub several: bool,
}

impl MemberReads {
    /// What `provenance` read, for `caller`.
    #[must_use]
    pub fn of(provenance: Option<&Provenance>, caller: Uuid) -> Self {
        let Some(provenance) = provenance else {
            return Self::default();
        };
        let others: Vec<Uuid> = provenance
            .members_drawn_on()
            .into_iter()
            .filter(|member| *member != caller)
            .collect();
        // A lone member read other than through the caller's coach view (a
        // group-mate's activities, a clubmate's quoted words) is no athlete
        // the caller may name: naming them fails the coaching gates, naming
        // anyone else draws on them.
        let lone_is_coached = match others.as_slice() {
            [lone] => provenance.members_drawn_on_as_coach(caller).contains(lone),
            _ => false,
        };
        Self {
            beyond_room: read_another_member(Some(provenance), caller),
            any: !others.is_empty(),
            several: provenance.drew_on_unknown_member()
                || others.len() > 1
                || (others.len() == 1 && !lone_is_coached),
        }
    }

    /// Every read made: what a turn whose caller cannot be read is taken to
    /// have read, so nothing is offered that a read would refuse.
    #[must_use]
    pub const fn every_read() -> Self {
        Self {
            beyond_room: true,
            any: true,
            several: true,
        }
    }

    /// Whether a write that names an athlete may still be taken after these
    /// reads, for some athlete.
    ///
    /// A structured one always. Free text while at most one other member
    /// was drawn on: a write onto that one member is taken, and dispatch
    /// refuses one naming anyone else, saying why
    /// ([`after_member_read_athlete_refusal`]).
    #[must_use]
    pub(crate) const fn takes_a_named_write(self, kind: WriteKind) -> bool {
        match kind {
            WriteKind::Structured => true,
            WriteKind::FreeText => !self.several,
        }
    }

    /// Whether a write of `kind` to the caller's own account is refused
    /// after these reads, `None` for a call that writes nothing there.
    ///
    /// Free text a later prompt reads is refused once the topic drew on any
    /// other member, as fact extraction is; a structured value only once it
    /// read one wider than the room's own view (carnet#1511 calls 16, 24).
    #[must_use]
    pub(crate) fn refuses(self, kind: Option<WriteKind>) -> bool {
        match kind {
            None => false,
            Some(WriteKind::FreeText) => self.any,
            Some(WriteKind::Structured) => self.beyond_room,
        }
    }
}

/// The refusal of a call to `tool_name` with `args` that would write the
/// caller's own account on a turn that read another member's data, or
/// `None` when it may run.
///
/// The athlete-thread rule ([`TurnSeat::AthleteThread`]), on a turn about a
/// member held on the subject's own seat ([`TurnSeat::Subject`]; the
/// dispatch chokepoint and the listing ask it there alone, since the other
/// seats withhold these writes themselves): such a turn is about the member, so
/// "her FTP is 250" written through `set_physiology` would become the
/// caller's own, and reach every later turn that reads their account, a
/// room's included. A write that names the member (a coach's plan tool in
/// the app) acts on them and runs; one that names no one is refused. The
/// listing withholds the same writes ([`offer_after_member_read`]).
#[must_use]
pub fn after_member_read_refusal(
    registry: &ToolRegistry,
    tool_name: &str,
    args: &Value,
    reads: MemberReads,
) -> Option<UniversalResponse> {
    reads
        .refuses(writes_on_caller(
            registry,
            tool_name,
            names_an_athlete(args),
        ))
        .then(|| UniversalResponse {
            success: false,
            result: Some(json!({
                "error_code": MEMBER_READ_ERROR_CODE,
                "reason": "member_read",
            })),
            error: Some(after_member_read_message(tool_name)),
            metadata: None,
        })
}

/// The refusal of a free-text write onto `target`'s account after a read
/// of another member, `None` when it may run.
///
/// Refused once the topic drew on a member besides the caller and `target`
/// (carnet#1511 call 24, [`drew_on_others_than`]). Asked where the write's
/// scope resolves the athlete it names (`plan_scope::acting_for`), on every
/// seat and every round of the turn: the listing withholds the same writes
/// where no athlete could pass ([`MemberReads::takes_a_named_write`]), and a
/// tier that lists its tools once learns of a read made mid-turn here, from
/// the reason this refusal names (call 25).
#[must_use]
pub(crate) fn after_member_read_athlete_refusal(
    registry: &ToolRegistry,
    tool_name: &str,
    caller: Uuid,
    target: Uuid,
    provenance: Option<&Provenance>,
) -> Option<String> {
    let free_text = registry.get(tool_name).is_some_and(|tool| {
        tool.capabilities().contains(ToolCapabilities::WRITES_DATA)
            && registry
                .write_kind(tool_name)
                .unwrap_or(WriteKind::FreeText)
                == WriteKind::FreeText
    });
    if !free_text || !drew_on_others_than(provenance, caller, target) {
        return None;
    }
    Some(if drew_on_named_others_than(provenance, caller, target) {
        format!(
            "'{tool_name}' is not available in this topic: it read another group member's \
             data, so what it writes may be theirs, and nothing from it is written onto this \
             athlete's account. Open this athlete's own thread from Group info to write it \
             there, or start a new topic."
        )
    } else {
        format!(
            "'{tool_name}' is not available in this topic: it replays messages written before \
             Dravr recorded whose data each drew on, and one may hold another group member's \
             data, so nothing from it is written onto this athlete's account. Start a new \
             topic to write it."
        )
    })
}

/// Whether `args` name an athlete as every write body reads the argument: a
/// string that is not blank once trimmed (`coach_scope::resolve_calendar_scope`,
/// `plan_scope::resolve_plan_scope`). Any other value (`""`, `"  "`, `false`,
/// `0`, `{}`) names no one, and the body writes the caller's own account.
pub(crate) fn names_an_athlete(args: &Value) -> bool {
    args.get(ATHLETE_ARGUMENT)
        .and_then(Value::as_str)
        .is_some_and(|name| !name.trim().is_empty())
}

/// What a turn that read `reads` offers of the tool `schema` declares: the
/// listing's half of [`after_member_read_refusal`], so the two accept the
/// same calls (carnet#1511 I7). `false` withholds the tool.
///
/// A write those reads refuse when it names no athlete is withheld when it
/// cannot name one as offered, and offered with `athlete` required when it
/// can: dispatch takes it only naming someone. Every other tool is offered
/// as it is. Applying it twice changes nothing, so a set built under these
/// reads stays byte-stable when they are applied again.
#[must_use]
pub(crate) fn offer_after_member_read(
    registry: &ToolRegistry,
    schema: &mut ToolSchema,
    reads: MemberReads,
) -> bool {
    let Some(kind) = writes_on_caller(registry, &schema.name, false) else {
        return true;
    };
    if !reads.refuses(Some(kind)) {
        return true;
    }
    let offers_athlete = reads.takes_a_named_write(kind)
        && schema
            .input_schema
            .properties
            .as_ref()
            .is_some_and(|properties| properties.contains_key(ATHLETE_ARGUMENT));
    if offers_athlete {
        let required = schema.input_schema.required.get_or_insert_with(Vec::new);
        if !required.iter().any(|name| name == ATHLETE_ARGUMENT) {
            required.push(ATHLETE_ARGUMENT.to_owned());
        }
    }
    offers_athlete
}

/// [`offer_after_member_read`] over a declaration as the model receives it:
/// the turn's set narrowed by a read a tool made after it was built.
#[must_use]
pub fn offer_declaration_after_member_read(
    registry: &ToolRegistry,
    declaration: &mut FunctionDeclaration,
    reads: MemberReads,
) -> bool {
    let Some(kind) = writes_on_caller(registry, &declaration.name, false) else {
        return true;
    };
    if !reads.refuses(Some(kind)) {
        return true;
    }
    if !reads.takes_a_named_write(kind) {
        return false;
    }
    let Some(parameters) = declaration.parameters.as_mut() else {
        return false;
    };
    if !takes_athlete_argument(parameters) {
        return false;
    }
    let Some(object) = parameters.as_object_mut() else {
        return false;
    };
    let required = object
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(required) = required.as_array_mut() {
        if !required.iter().any(|name| name == ATHLETE_ARGUMENT) {
            required.push(Value::String(ATHLETE_ARGUMENT.to_owned()));
        }
    }
    *parameters = to_canonical_value(&*parameters);
    true
}

/// `error_code` of the refusal of a write to the caller's own account on a
/// turn that read another member's data.
pub(crate) const MEMBER_READ_ERROR_CODE: &str = "member_read_withheld";

/// What the model reads when it calls a write to the caller's own account on
/// a turn that read another member's data.
fn after_member_read_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available on this turn: this conversation read another \
         athlete's data, so what it says may be theirs, and nothing from it is saved to the \
         caller's own account — not as a fact, goal, plan, place or setting. If the caller \
         wants to change their own data, tell them to start a new topic about themselves."
    )
}

/// Whether `tool_name` reads the caller's own data: it reads data, declares
/// no data class, and takes no [`ATHLETE_ARGUMENT`] to read anyone else.
fn reads_callers_own_data(registry: &ToolRegistry, tool_name: &str) -> bool {
    let Some(tool) = registry.get(tool_name) else {
        return false;
    };
    tool.capabilities().contains(ToolCapabilities::READS_DATA)
        && registry.data_class(tool_name).is_none()
        && !takes_athlete_argument(&tool.definition().input_schema)
}

/// Whether `tool_name` acts on the athlete a thread is bound to: it takes
/// [`ATHLETE_ARGUMENT`], which in such a thread defaults to its athlete and
/// names no one else (the plan, calendar and library tools).
fn acts_on_the_threads_athlete(registry: &ToolRegistry, tool_name: &str) -> bool {
    registry
        .get(tool_name)
        .is_some_and(|tool| takes_athlete_argument(&tool.definition().input_schema))
}

fn takes_athlete_argument(input_schema: &Value) -> bool {
    input_schema
        .get("properties")
        .and_then(|properties| properties.get(ATHLETE_ARGUMENT))
        .is_some()
}

/// What the model reads when it calls a tool withheld on the coach's seat.
fn coach_seat_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available here: you are talking to this group's coach, and \
         their own data is never read in the group. For an athlete in the group, call \
         get_group_member_activities with that athlete's name for their sessions, or \
         get_group_member_plan_status for whether they have a published plan and where their \
         coach reads it — the plan itself is never posted in the group. If the coach asks \
         about their own training, tell them to ask in their own conversation with Dravr, or \
         in a group where they train as an athlete."
    )
}

/// What the model reads when it calls a read withheld in a coach's thread
/// about one athlete.
fn athlete_thread_read_message(tool_name: &str) -> String {
    let wellness = read_subject::wellness_data();
    format!(
        "'{tool_name}' is not available here: in this thread about one athlete it would read the \
         coach's own data, never the athlete's. The athlete's training reads here — activities, \
         the daily load history, trends, goals, training physiology and the analyses of them — \
         act on the athlete with `athlete` left out. Their wellness ({wellness}) reads here \
         only once they share it with the coach, which only they can do, \
         {WELLNESS_GRANT_WHERE}. Their memory is never readable here; \
         say so. If the coach wants their own data, tell them to ask in their own conversation \
         with Dravr."
    )
}

/// What the model reads when it calls a tool withheld in a thread about an
/// athlete the caller no longer reaches.
fn unreached_thread_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available here: this thread is about an athlete the coach no \
         longer coaches here, or who stopped sharing with them, so nothing of theirs is read or \
         written from it. If the coach wants to work on their own plan or calendar, tell them \
         to ask in their own conversation with Dravr."
    )
}

/// What the model reads when it calls a write withheld in a coach's thread
/// about one athlete.
fn athlete_thread_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available here: this thread is the coach's workspace for one \
         athlete's plan, and nothing said in it is saved to the coach's own account — not as \
         a fact, note, follow-up, commitment, goal or setting. The athlete's plan and calendar \
         tools act on the athlete. If the coach wants to change their own data, tell them to \
         do it in their own conversation with Dravr."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::CarriedRead;
    use pierre_providers::ai_scope;

    /// carnet#1511 call 16: free text a later prompt reads is refused once a
    /// turn drew on any other member, a structured own-account value only
    /// once it read one wider than its room's own view, and a call that
    /// writes nothing on the caller never.
    #[test]
    fn a_write_is_refused_by_what_it_stores() {
        let room_visible = MemberReads {
            beyond_room: false,
            any: true,
            several: false,
        };
        let beyond_room = MemberReads {
            beyond_room: true,
            any: true,
            several: false,
        };
        let none = MemberReads::default();
        assert!(
            room_visible.refuses(Some(WriteKind::FreeText)),
            "free text is refused once a turn drew on another member"
        );
        assert!(
            !room_visible.refuses(Some(WriteKind::Structured)),
            "a structured value is taken after a read within the room's own view"
        );
        assert!(
            beyond_room.refuses(Some(WriteKind::FreeText)),
            "free text is refused after a read beyond the room"
        );
        assert!(
            beyond_room.refuses(Some(WriteKind::Structured)),
            "a structured value is refused after a read beyond the room"
        );
        for reads in [none, room_visible, beyond_room] {
            assert!(
                !reads.refuses(None),
                "a call that writes nothing on the caller is never refused: {reads:?}"
            );
        }
        assert!(
            !none.refuses(Some(WriteKind::FreeText)),
            "free text is taken when the turn drew on no one"
        );
        assert!(
            !none.refuses(Some(WriteKind::Structured)),
            "a structured value is taken when the turn drew on no one"
        );
    }

    /// carnet#1511 calls 24 and 26: what a topic drew on is decided from its
    /// provenance, a read replayed from its earlier turns counted as one
    /// made on this turn. After one other member, a free-text write naming
    /// an athlete is still taken (for that member alone, dispatch decides);
    /// after several, for none. A write's own target and the caller's own
    /// data draw on no one.
    #[tokio::test]
    async fn member_reads_count_the_whole_topic_and_no_write_target() {
        let (caller, alex, bea) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let reads_after = |reads: Vec<(CarriedRead, &'static str)>| async move {
            let provenance = Provenance::new();
            ai_scope::tracking(provenance.clone(), async {
                for (read, how) in reads {
                    match how {
                        "replayed" => ai_scope::note_replayed_member_read(read),
                        "written" => ai_scope::note_member_written(read),
                        _ => ai_scope::note_member_read(read),
                    }
                }
            })
            .await;
            provenance
        };

        let several = reads_after(vec![
            (CarriedRead::peer(alex, None), "replayed"),
            (CarriedRead::peer(bea, None), "replayed"),
        ])
        .await;
        let reads = MemberReads::of(Some(&several), caller);
        assert!(
            reads.several && reads.any,
            "two replayed reads count as several members: {reads:?}"
        );
        assert!(
            !reads.takes_a_named_write(WriteKind::FreeText),
            "bug: free text naming an athlete is listed after the topic read two members"
        );
        assert!(
            drew_on_others_than(Some(&several), caller, alex),
            "bug: a roster read replayed from an earlier turn let free text onto Alex"
        );

        let one = reads_after(vec![(CarriedRead::coach(bea, Uuid::nil(), caller), "live")]).await;
        let reads = MemberReads::of(Some(&one), caller);
        assert!(
            reads.takes_a_named_write(WriteKind::FreeText),
            "free text naming an athlete is taken after one coached member's read"
        );
        assert!(
            reads.takes_a_named_write(WriteKind::Structured),
            "a structured named write is taken after one coached member's read"
        );
        assert!(
            drew_on_others_than(Some(&one), caller, alex),
            "a read of Bea draws on someone other than Alex"
        );

        // A lone member the caller read other than as their coach (a
        // group-mate's activities, a clubmate's quoted words) is no athlete
        // the caller may name: a free-text write naming anyone is refused,
        // so none is offered.
        let peer = reads_after(vec![(CarriedRead::peer(bea, None), "live")]).await;
        let reads = MemberReads::of(Some(&peer), caller);
        assert!(
            !reads.takes_a_named_write(WriteKind::FreeText),
            "bug: free text naming an athlete is offered after a peer's read every call refuses"
        );
        assert!(
            reads.takes_a_named_write(WriteKind::Structured),
            "a structured named write is still taken after a peer's read"
        );
        assert!(
            !drew_on_others_than(Some(&one), caller, bea),
            "bug: a read of Bea refused a write onto Bea herself"
        );

        let written = reads_after(vec![
            (CarriedRead::coach(alex, Uuid::nil(), caller), "written"),
            (CarriedRead::coach(alex, Uuid::nil(), caller), "written"),
        ])
        .await;
        assert_eq!(
            MemberReads::of(Some(&written), caller),
            MemberReads::default(),
            "bug: a write onto Alex counted as drawing on her"
        );
        assert!(
            !drew_on_others_than(Some(&written), caller, bea),
            "writes onto Alex draw on no one"
        );

        let own = reads_after(vec![(CarriedRead::peer(caller, None), "live")]).await;
        assert_eq!(
            MemberReads::of(Some(&own), caller),
            MemberReads::default(),
            "the caller's own data draws on no member"
        );
    }

    /// carnet#1511 A8: a replayed legacy row whose reads no row names draws
    /// on members no one can tell. A free-text write onto any athlete is
    /// refused (none is offered naming one), while the caller's own account
    /// keeps every write: it read no one wider than its room, and drew on no
    /// member a read names.
    #[tokio::test]
    async fn an_unknown_member_refuses_free_text_onto_any_athlete_only() {
        let (caller, alex) = (Uuid::new_v4(), Uuid::new_v4());
        let provenance = Provenance::new();
        ai_scope::tracking(provenance.clone(), async {
            ai_scope::note_replayed_unknown_member();
        })
        .await;
        assert!(
            provenance.drew_on_unknown_member(),
            "a replayed legacy row draws on an unknown member"
        );
        let reads = MemberReads::of(Some(&provenance), caller);
        assert!(
            reads.several && !reads.any && !reads.beyond_room,
            "bug: an unknown member is read as {reads:?}"
        );
        assert!(
            !reads.takes_a_named_write(WriteKind::FreeText),
            "free text naming an athlete is refused after an unknown member's read"
        );
        assert!(
            reads.takes_a_named_write(WriteKind::Structured),
            "a structured named write is taken after an unknown member's read"
        );
        assert!(
            !reads.refuses(Some(WriteKind::FreeText)),
            "the caller's own account keeps a free-text write"
        );
        assert!(
            !reads.refuses(Some(WriteKind::Structured)),
            "the caller's own account keeps a structured write"
        );
        assert!(
            drew_on_others_than(Some(&provenance), caller, alex),
            "an unknown member may be anyone other than Alex"
        );
        assert!(
            !drew_on_named_others_than(Some(&provenance), caller, alex),
            "an unknown member is no member a read names"
        );
        assert!(
            !drew_on_others_than(Some(&Provenance::new()), caller, alex),
            "a turn that read no one draws on no one"
        );
    }

    #[test]
    fn the_coach_seat_refusal_points_at_the_athlete_tool_and_the_coachs_own_thread() {
        let message = coach_seat_message("get_activities");
        assert!(
            message.contains("get_group_member_activities"),
            "the refusal points at the athlete tool: {message}"
        );
        assert!(
            message.contains("their own conversation with Dravr"),
            "the refusal points at the coach's own thread: {message}"
        );
    }

    #[test]
    fn the_coach_seat_refusal_points_a_plan_request_at_the_plan_status() {
        let message = coach_seat_message("get_training_plan");
        assert!(
            message.contains("get_group_member_plan_status"),
            "the refusal names the plan-status tool a coach seat keeps: {message}"
        );
        assert!(
            message.contains("the plan itself is never posted in the group"),
            "the refusal says the plan stays out of the room: {message}"
        );
    }

    #[test]
    fn the_athlete_thread_refusal_says_nothing_is_saved_to_the_coach() {
        let message = athlete_thread_message("remember_fact");
        assert!(message.starts_with("'remember_fact' is not available here"));
        assert!(message.contains("nothing said in it is saved to the coach's own account"));
        assert!(message.contains("their own conversation with Dravr"));
    }

    #[test]
    fn the_athlete_thread_read_refusal_says_whose_data_it_would_read() {
        let message = athlete_thread_read_message("recall_user_memory");
        assert!(
            message.starts_with("'recall_user_memory' is not available here"),
            "bug: the refusal names the tool it withholds: {message}"
        );
        assert!(
            message.contains("the coach's own data, never the athlete's"),
            "bug: the refusal says whose data the read would return: {message}"
        );
        assert!(
            message.contains("Their memory is never readable here; say so"),
            "bug: the refusal tells the model to say so rather than estimate: {message}"
        );
        assert!(
            message.contains(
                "Their wellness (sleep, HRV, resting heart rate, weight, readiness and daily \
                 notes) reads here only once they share it with the coach"
            ) && message.contains("Group info → What your human coach can do"),
            "bug: the refusal says the wellness takes the athlete's grant, and where: {message}"
        );
    }

    #[test]
    fn only_a_property_named_athlete_counts_as_acting_on_someone_else() {
        assert!(takes_athlete_argument(&json!({
            "type": "object",
            "properties": { "athlete": { "type": "string" }, "weeks": { "type": "array" } },
        })));
        assert!(!takes_athlete_argument(&json!({
            "type": "object",
            "properties": { "athlete_notes": { "type": "string" } },
        })));
        assert!(!takes_athlete_argument(&json!({ "type": "object" })));
    }

    #[cfg(feature = "tools-groups")]
    mod with_the_plan_status {
        use std::sync::Arc;

        use super::super::*;
        use crate::implementations::group_member_plan::GetGroupMemberPlanStatusTool;
        use crate::implementations::training_plans::create_training_plan_tools;

        #[test]
        fn the_coach_seat_keeps_the_plan_status_and_withholds_the_plan() {
            let mut registry = ToolRegistry::new();
            registry.register_with_category(Arc::new(GetGroupMemberPlanStatusTool), "groups");
            for tool in create_training_plan_tools() {
                registry.register_with_category(Arc::from(tool), "memory");
            }
            assert!(
                !TurnSeat::Coach.withholds(&registry, "get_group_member_plan_status"),
                "a groups tool stays callable on the coach seat"
            );
            assert!(
                TurnSeat::Coach.withholds(&registry, "get_training_plan"),
                "the plan itself stays withheld on the coach seat"
            );
        }
    }

    #[cfg(feature = "tools-memory")]
    mod with_registered_tools {
        use std::sync::Arc;

        use super::super::*;
        use crate::implementations::memory::create_memory_tools;
        use crate::implementations::training_plans::create_training_plan_tools;

        fn registry() -> ToolRegistry {
            let mut registry = ToolRegistry::new();
            for tool in create_memory_tools()
                .into_iter()
                .chain(create_training_plan_tools())
            {
                registry.register_with_category(Arc::from(tool), "memory");
            }
            registry
        }

        #[test]
        fn the_athlete_thread_withholds_every_write_on_the_coachs_own_account() {
            let registry = registry();
            let own_write = "remember_fact";
            assert!(
                TurnSeat::AthleteThread.withholds(&registry, own_write),
                "{own_write} files on the caller's own account"
            );
            let refusal = TurnSeat::AthleteThread
                .refusal(&registry, own_write)
                .expect("a withheld tool is refused");
            assert!(!refusal.success, "a withheld tool's refusal is no success");
            assert_eq!(
                refusal.result.unwrap()["error_code"],
                ATHLETE_THREAD_ERROR_CODE,
                "a withheld tool is refused with the athlete-thread error code"
            );
        }

        #[test]
        fn the_athlete_thread_keeps_the_plan_tools_and_withholds_the_coachs_own_reads() {
            let registry = registry();
            // The plan tools take `athlete`, which defaults to the thread's
            // athlete.
            for kept in ["save_training_plan", "get_training_plan"] {
                assert!(
                    !TurnSeat::AthleteThread.withholds(&registry, kept),
                    "{kept}"
                );
                assert!(
                    TurnSeat::AthleteThread.refusal(&registry, kept).is_none(),
                    "the athlete thread does not refuse the plan tool {kept}"
                );
            }
            // Recall reads only the coach's own memory: it would answer a
            // question about the athlete with the coach's facts.
            let refusal = TurnSeat::AthleteThread
                .refusal(&registry, "recall_user_memory")
                .expect("the coach's own read is withheld in the thread");
            assert_eq!(
                refusal.result.unwrap()["reason"],
                "athlete_thread_read",
                "a read is refused with the read's own wording"
            );
        }

        #[cfg(feature = "tools-mobility")]
        #[test]
        fn a_declared_data_class_keeps_a_read_in_the_athlete_thread() {
            use crate::implementations::mobility::create_mobility_tools;
            use crate::read_subject::DataClass;
            use dravr_tronc::mcp::tool::ToolCapabilities;

            let mut registry = registry();
            for tool in create_mobility_tools() {
                registry.register_with_category(Arc::from(tool), "mobility");
            }
            let reference = "list_yoga_poses";
            assert_eq!(
                registry.data_class(reference),
                Some(DataClass::NoSubject),
                "premise: {reference} declares a class"
            );
            assert!(
                registry
                    .get(reference)
                    .is_some_and(|tool| tool.capabilities().contains(ToolCapabilities::READS_DATA)),
                "premise: {reference} reads data"
            );
            assert!(
                !TurnSeat::AthleteThread.withholds(&registry, reference),
                "a read that declares its class answers the same whoever asks"
            );
        }

        // carnet#1511 call 30: a thread whose athlete the coach no longer
        // reaches is the coach's private seat, save the tools bound to the
        // thread's athlete, which every call would refuse.
        #[test]
        fn an_unreached_thread_withholds_only_the_tools_bound_to_its_athlete() {
            let registry = registry();
            for bound in ["save_training_plan", "get_training_plan"] {
                assert!(
                    TurnSeat::UnreachedThread.withholds(&registry, bound),
                    "bug: {bound} is offered where every call is refused"
                );
                assert_eq!(
                    TurnSeat::UnreachedThread
                        .refusal(&registry, bound)
                        .unwrap()
                        .result
                        .unwrap()["reason"],
                    "thread_unreached"
                );
            }
            for own in ["remember_fact", "recall_user_memory"] {
                assert!(
                    !TurnSeat::UnreachedThread.withholds(&registry, own),
                    "{own}"
                );
            }
            assert!(TurnSeat::UnreachedThread.sender_is_subject());
        }

        // carnet#1511 call 30: the grant taint refuses what a turn's topic
        // refuses, by the one rule: a declared free-text write, never a
        // read or a structured value, and a write that declared no kind is
        // free text.
        #[test]
        fn the_grant_taint_reads_free_text_by_the_seats_rule() {
            let registry = registry();
            assert!(files_free_text(&registry, "remember_fact"));
            assert!(!files_free_text(&registry, "recall_user_memory"));
            assert!(!files_free_text(&registry, "not_a_tool"));
            for name in registry.tool_names() {
                assert_eq!(
                    files_free_text(&registry, name),
                    writes_on_caller(&registry, name, false) == Some(WriteKind::FreeText),
                    "{name}"
                );
            }
        }

        #[test]
        fn the_subject_seat_withholds_nothing_and_the_coach_seat_every_data_tool() {
            let registry = registry();
            for name in ["remember_fact", "save_training_plan", "recall_user_memory"] {
                assert!(!TurnSeat::Subject.withholds(&registry, name), "{name}");
                assert!(TurnSeat::Subject.refusal(&registry, name).is_none());
                assert!(TurnSeat::Coach.withholds(&registry, name), "{name}");
                assert_eq!(
                    TurnSeat::Coach
                        .refusal(&registry, name)
                        .unwrap()
                        .result
                        .unwrap()["error_code"],
                    COACH_SEAT_ERROR_CODE
                );
            }
        }
    }
}
