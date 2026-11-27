// ABOUTME: What a listing beside an executor asks of the turn it serves: its role, its room, and the members it read
// ABOUTME: Read from the same conversation, tenant, audience and provenance the executor's own calls are decided by

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#[cfg(feature = "client-chat")]
use std::borrow::Cow;

use pierre_core::models::TenantId;
#[cfg(feature = "client-chat")]
use pierre_llm::Tool;
use pierre_providers::ai_scope;
use tracing::warn;
use uuid::Uuid;

use super::UniversalExecutor;
use crate::chat_catalogue::TurnReaders;
use crate::coach_seat;
use crate::implementations::call_room::{turn_room, TurnAudience};
use crate::tool_access::CallerRole;

impl UniversalExecutor {
    /// What the turn this executor serves read of members other than
    /// `caller` ([`coach_seat::MemberReads`]), from the provenance it
    /// carries, else the one running here.
    #[must_use]
    pub fn member_reads(&self, caller: Uuid) -> coach_seat::MemberReads {
        let provenance = self
            .provenance
            .clone()
            .or_else(ai_scope::current_provenance);
        coach_seat::MemberReads::of(provenance.as_ref(), caller)
    }

    /// `tools`, the declarations the turn was offered when it started,
    /// narrowed by what it has read since ([`coach_seat::offer_declaration_after_member_read`]):
    /// what the next model call of the in-process tool loop is offered, so a
    /// member read made mid-turn withholds the writes dispatch would refuse
    /// from then on, as the loopback listing does per call (carnet#1511 I7).
    ///
    /// The set the turn started with when nothing it read refuses anything,
    /// off the subject's own seat (whose seats withhold those writes
    /// themselves), or for a caller that is not a user id; the narrowing is
    /// idempotent, so a set built under the same reads comes back unchanged.
    #[must_use]
    #[cfg(feature = "client-chat")]
    pub(crate) fn offered_now<'tools>(
        &self,
        tools: &'tools Tool,
        caller: &str,
    ) -> Cow<'tools, Tool> {
        let Ok(caller) = Uuid::parse_str(caller) else {
            return Cow::Borrowed(tools);
        };
        let reads = self.member_reads(caller);
        if !self.seat.sender_is_subject() || reads == coach_seat::MemberReads::default() {
            return Cow::Borrowed(tools);
        }
        let registry = self.resources.tool_registry();
        let mut narrowed = tools.clone();
        narrowed.function_declarations.retain_mut(|declaration| {
            coach_seat::offer_declaration_after_member_read(registry, declaration, reads)
        });
        Cow::Owned(narrowed)
    }

    /// The role the turn this executor serves was decided for: what a
    /// listing beside the executor must offer by. A caller with no role when
    /// none was bound.
    #[must_use]
    pub fn turn_role(&self) -> CallerRole {
        self.turn_role.unwrap_or_default()
    }

    /// Who reads what this executor's calls write, for `caller` under
    /// `caller_tenant`: the room a call naming an athlete is decided in,
    /// read from the same conversation, tenant and audience dispatch reads
    /// it from. What a listing beside the executor offers the `athlete`
    /// argument by, so it offers none a call would be refused. A room that
    /// cannot be read is taken for one others read.
    pub async fn turn_readers(&self, caller: Uuid, caller_tenant: Option<Uuid>) -> TurnReaders {
        let room = turn_room(
            self.resources.repos(),
            TurnAudience {
                caller,
                conversation_id: self.conversation_id.as_deref(),
                conversation_tenant: self.conversation_tenant_id.map(TenantId::from_uuid),
                caller_tenant: caller_tenant.map(TenantId::from_uuid),
                direct_message: self.turn_direct_message,
            },
        )
        .await;
        match room {
            Ok(room) => TurnReaders::of_room(self.resources.repos(), &room, caller).await,
            Err(e) => {
                warn!(error = %e, "chat tools: the turn's room could not be read; offering no `athlete`");
                TurnReaders::Others
            }
        }
    }
}
