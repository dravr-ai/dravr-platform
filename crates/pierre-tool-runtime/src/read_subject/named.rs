// ABOUTME: Whose data a read reads when an outside caller names a coached athlete with `athlete=`
// ABOUTME: Taken only where the caller alone reads the reply, then matched and gated as every coached read is

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coached athlete named with `athlete=`
//!
//! MCP, A2A and the chat turns served outside Dravr's own app may name a
//! coached athlete (carnet#1511, Q16). The name is matched among the members
//! who share with the caller, then gated by id as a thread's athlete is.
//! Before either, the room rule: a name is taken only where the caller alone
//! reads the reply (carnet#1422).

use pierre_core::errors::AppResult;
use pierre_services::coached_athlete::CoachedAthleteRefusal;

use super::{
    coached_read, match_coached_member, refusal, CoachedReadRefusal, MatchAudience, MemberMiss,
    ReadSubject, ReadSubjectRequest, SubjectSource,
};
use crate::implementations::call_room::{turn_room, TurnAudience};
use crate::protocol::types::UniversalResponse;

/// The subject an external caller named with `athlete=`: the one coached
/// member the name matches, once the coaching gates let the caller read them.
pub(super) async fn named_subject(
    request: &ReadSubjectRequest<'_>,
    name: &str,
) -> AppResult<Result<Option<ReadSubject>, UniversalResponse>> {
    const YOURSELF: &str = "omit `athlete` to read your own data.";
    let repos = request.resources.repos();
    // Named only outside Dravr's own app (MCP, A2A, an API key's chat): the
    // caller is an outside assistant, never told of a member who does not
    // share with the coach.
    let audience = MatchAudience::OutsideAssistant;
    let no_match = || MemberMiss::NoSharingMatch.message(name, YOURSELF);
    let Some(requester_tenant) = request.requester_tenant else {
        return Ok(Err(refusal("no_match", no_match())));
    };
    if let Some(refused) = room_refusal(request).await? {
        return Ok(Err(refused));
    }
    let found = match match_coached_member(repos, request.requester, name, None, audience).await? {
        Ok(found) => found,
        Err(miss) => return Ok(Err(refusal(miss.reason(), miss.message(name, YOURSELF)))),
    };
    // Gates 2 to 4, by id. The match prefers a group where the athlete shares
    // with the coach, so a consent refusal names the most permissive row; a
    // membership that changed since the match reads as no match.
    match coached_read(
        repos,
        request.requester,
        found.user_id,
        found.group.id,
        request.class,
    )
    .await?
    {
        Ok(access) => Ok(Ok(Some(ReadSubject::new(
            request,
            requester_tenant,
            access,
            SubjectSource::ExternalArgument,
        )))),
        // Sharing turned off since the match: said as no match, so the
        // member is not named to the assistant.
        Err(CoachedReadRefusal::Coaching(CoachedAthleteRefusal::CoachSharingRevoked {
            ..
        })) => Ok(Err(refusal("no_match", no_match()))),
        Err(refused) => Ok(Err(refused.response(request.tool_name, |_| no_match()))),
    }
}

/// The refusal for a name sent where others may read the reply, if it was:
/// the room rule `plan_scope`'s gate 1 and `coach_scope::room_refusal_in`
/// apply, asked of the one classifier ([`turn_room`], carnet#1422). A chat
/// turn served over an API key may run in a group's room, and its reply is
/// fanned out to everyone in it, so a name is taken only where the caller
/// alone reads the reply: no conversation (MCP, A2A), or one confirmed
/// private. In the room of a group the caller coaches the coach seat already
/// refuses a data read and points at the group tools, which pin the room.
async fn room_refusal(request: &ReadSubjectRequest<'_>) -> AppResult<Option<UniversalResponse>> {
    let room = turn_room(
        request.resources.repos(),
        TurnAudience {
            caller: request.requester,
            conversation_id: request.conversation_id,
            conversation_tenant: request.conversation_tenant,
            caller_tenant: request.requester_tenant,
            direct_message: request.direct_message,
        },
    )
    .await?;
    Ok((!room.is_private()).then(|| {
        refusal(
            "group_room",
            "Name an athlete from your own conversation with Dravr — a room is read by everyone \
             in it, and this conversation is not confirmed as private to you."
                .to_owned(),
        )
    }))
}
