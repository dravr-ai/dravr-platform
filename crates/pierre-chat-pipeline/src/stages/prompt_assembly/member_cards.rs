// ABOUTME: The member data the platform itself puts into a turn's prompt, read before the tool index
// ABOUTME: Each member it carries is noted as a member read, so the reply is stored about them (carnet#1511)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Member context the prompt carries without any tool reading it.
//!
//! Three sources put another member's data into a turn's prompt on the
//! platform's own initiative:
//!
//! - **A group room's cards** ([`resolve_group_context`] and
//!   `GroupService::inject_group_context`): each member who shares with the
//!   sender, the coach's view for the group's coach.
//! - **A coach's thread about one athlete** (`subject_athlete::block`): the
//!   athlete's roster line, plan, draft and linked wellness.
//! - **A messaging group chat's recent room words** (the ambient block the
//!   ingress builds with the room's view): each member it quotes.
//!
//! What the turn derives from that context may restate it, exactly as it
//! may restate a tool's read. So each member carried, the sender aside, is
//! noted on the turn's provenance ([`ai_scope::note_member_read`]) the
//! moment their data is read: the reply is stored about them, and every
//! reader of the row, an agent draft's excerpt, a compaction summary and
//! fact extraction then read it under the rule for that member (carnet#1511
//! design call 4).
//!
//! A guided walk bound to the sender interviews them about themselves, so
//! its turns carry no room card and no room words: read there, they would
//! make every answer a member read, and the walk would file nothing on the
//! sender.
//!
//! It is read here, ahead of the tool index, because the index offers what
//! the turn's dispatch will accept, and a turn that read another member
//! withholds writes on the sender's own account (carnet#1422). Read after
//! the index, the index would offer a write that dispatch then refuses. The
//! group block itself is appended later, by prompt assembly.

use std::collections::BTreeSet;

use pierre_core::errors::AppResult;
use pierre_core::models::{CarriedRead, MemberFitnessSnapshot};
use pierre_providers::ai_scope;
use uuid::Uuid;

use super::super::subject_athlete::SubjectTurn;
use crate::turn::AmbientContext;
#[cfg(feature = "tools-groups")]
use crate::turn::TurnInput;
#[cfg(feature = "tools-groups")]
use crate::ChatPipelineContext;
#[cfg(feature = "tools-groups")]
use pierre_database::database::ConversationRecord;

#[cfg(feature = "tools-groups")]
use super::super::prompt_builder::{card_scope, resolve_group_context};
#[cfg(feature = "tools-groups")]
#[cfg(feature = "tools-groups")]
use pierre_groups::service::MemberCards;

/// The member context a turn's prompt carries.
pub(super) struct CarriedMembers {
    /// The group block, appended to the prompt where the group context goes;
    /// empty outside a group conversation.
    pub block: String,
    /// The group roster snapshots peer grounding and the peer-claim verifier
    /// match names against.
    pub roster: Vec<MemberFitnessSnapshot>,
    /// Every member whose data the prompt carries: the room's cards shown to
    /// the sender, and on a coach's athlete thread the athlete.
    pub carried: BTreeSet<Uuid>,
}

/// Who a turn's member context is read for.
pub(super) struct CardReader<'turn> {
    /// The sender.
    pub sender: Uuid,
    /// The messaging group chat's recent room transcript the prompt carries,
    /// when it does: the members it quotes are read as its cards are.
    pub ambient: Option<&'turn AmbientContext>,
    /// The coach's athlete thread the turn runs in, when it does.
    pub subject_turn: Option<&'turn SubjectTurn>,
    /// What the group room's cards are read from.
    #[cfg(feature = "tools-groups")]
    pub room: RoomReader<'turn>,
}

/// What a turn's group room cards are read from.
#[cfg(feature = "tools-groups")]
pub(super) struct RoomReader<'turn> {
    /// Shared pipeline context.
    pub ctx: &'turn ChatPipelineContext,
    /// The turn.
    pub input: &'turn TurnInput,
    /// Its conversation.
    pub conv: &'turn ConversationRecord,
    /// Whether the sender is the turn's own subject (off the coach's seat
    /// and the coach's athlete thread).
    pub sender_is_subject: bool,
    /// Whether the sender holds the group's coach seat.
    pub coach_seat: bool,
    /// Whether the turn answers a guided walk bound to the sender: an
    /// interview about them alone, which carries no other member's card, so
    /// its answers stay theirs to file.
    pub walks_the_sender: bool,
}

/// Read the member context `reader`'s prompt carries, noting every member
/// it carries other than the sender as read.
///
/// # Errors
///
/// Returns the repository error the coach's gate or a read's record raises
/// ([`resolve_group_context`]).
pub(super) async fn carry(reader: CardReader<'_>) -> AppResult<CarriedMembers> {
    // Each card is read the way the sender reads it: through the coach's
    // view on the coach's seat, else through the room's peer view
    // (carnet#1511 call 19).
    #[cfg(feature = "tools-groups")]
    let (block, roster, mut carried, mut reads) = if reader.room.walks_the_sender {
        (String::new(), Vec::new(), BTreeSet::new(), Vec::new())
    } else {
        let (block, roster, carried, group) = group_cards(&reader.room, reader.sender).await?;
        let reads = carried
            .iter()
            .map(|member| match group {
                Some(group) if reader.room.coach_seat => {
                    CarriedRead::coach(*member, group, reader.sender)
                }
                group => CarriedRead::peer(*member, group),
            })
            .collect();
        (block, roster, carried, reads)
    };
    // Without groups no room exists, and nothing of the room is read.
    #[cfg(not(feature = "tools-groups"))]
    let (block, roster, mut carried, mut reads) = (
        String::new(),
        Vec::new(),
        BTreeSet::new(),
        Vec::<CarriedRead>::new(),
    );
    // A coach's thread about one athlete carries her roster line, the same
    // snapshot a room card is built from, through the coach's view: her data
    // reaches the model whether or not a tool reads her.
    if let Some(SubjectTurn::Granted(granted)) = reader.subject_turn {
        carried.insert(granted.athlete());
        reads.push(granted.carried_read(reader.sender));
    }
    // A group chat's recent room words quote the members who wrote them,
    // and every read their entries carried (carnet#1511 design call 8).
    if let Some(ambient) = reader.ambient {
        carried.extend(ambient.members.iter().map(|read| read.member));
        reads.extend(ambient.members.iter().cloned());
    }
    for read in reads
        .into_iter()
        .filter(|read| read.member != reader.sender)
    {
        ai_scope::note_member_read(read);
    }
    Ok(CarriedMembers {
        block,
        roster,
        carried,
    })
}

/// The group room's cards: the block the prompt carries, the roster, and
/// whose cards the block holds.
#[cfg(feature = "tools-groups")]
async fn group_cards(
    reader: &RoomReader<'_>,
    sender: Uuid,
) -> AppResult<(
    String,
    Vec<MemberFitnessSnapshot>,
    BTreeSet<Uuid>,
    Option<Uuid>,
)> {
    let RoomReader {
        ctx, input, conv, ..
    } = *reader;
    // The group context always shows the requester their own card, which
    // for the coach would present the coach's training as the group's: on
    // the coach's seat it is never read.
    // A messaging group chat reads the reply as it is posted, every member
    // at once: the coach's cards there are the room's (carnet#1511 design
    // call 6).
    let room_reads_reply =
        ai_scope::posts_to_group_chat(ai_scope::declared_transport(), input.is_direct_message);
    let (resolved_group_id, snapshots) = resolve_group_context(
        ctx,
        conv.group_id.as_deref(),
        input.tool_tenant_id,
        sender,
        (!reader.sender_is_subject).then_some(sender),
        room_reads_reply,
    )
    .await?;
    // A coach's message about one athlete carries that athlete's card alone
    // (carnet#1464). The roster stays whole for peer grounding and the reply
    // checks, which match names against it.
    let shown = reader
        .coach_seat
        .then(|| card_scope(&input.content, &snapshots, sender))
        .flatten();
    let injected = ctx
        .group_service
        .inject_group_context(
            "",
            "",
            sender,
            input.tool_tenant_id,
            resolved_group_id.as_deref(),
            MemberCards {
                snapshots: &snapshots,
                shown: shown.as_ref(),
            },
        )
        .await;
    let group = resolved_group_id
        .as_deref()
        .and_then(|group| Uuid::parse_str(group).ok());
    Ok(match injected {
        Ok(injected) => (injected.prompt, snapshots, injected.cards_carried, group),
        Err(_) => (String::new(), snapshots, BTreeSet::new(), group),
    })
}
