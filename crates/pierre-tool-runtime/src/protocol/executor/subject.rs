// ABOUTME: Whose data a dispatched tool call reads: the caller's own, or a coached athlete's
// ABOUTME: Resolves the read subject, the parties it splits a call into, and the context built as the athlete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The read-subject half of the dispatch chokepoint (carnet#1387).
//!
//! A training- or wellness-class tool reads a coached athlete when the call
//! names one: the athlete of a coach's thread about them, or `athlete=` over
//! MCP and A2A. A wellness-class tool also needs their `coach_wellness_read`,
//! on either path. [`UniversalExecutor::read_subject`] resolves that
//! once, before the body runs; [`Parties`] then tells every later step whose
//! configuration, data sources and consents it asks (the athlete's) from
//! whose turn and charge it is (the coach's).

use std::sync::Arc;

use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_providers::{ai_scope, backend_resolver};
use serde_json::Value as JsonValue;
use tracing::info;
use uuid::Uuid;

use super::UniversalExecutor;
use crate::context::AuthMethod;
use crate::guardian;
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::read_subject::{
    self, resolve_read_subject, DataClass, ReadSubject, ReadSubjectRequest, ATHLETE_ARGUMENT,
};
use crate::reconnect::{RECONNECT_BACKEND_KEY, RECONNECT_REQUIRED_KEY};
use crate::runtime::ToolRuntime;

/// Who a call is for and whose data it reads.
///
/// The same person on every call but one: a coach's read of a coached
/// athlete ([`ReadSubject`]) is asked for by the coach, who is charged for it
/// and whose turn it belongs to, and reads the athlete, whose tenant, data
/// sources and AI consents decide what it may read.
#[derive(Debug, Clone, Copy)]
pub(super) struct Parties {
    /// Who asked: their tool configuration, Guardian turn and charge.
    pub(super) requester: Uuid,
    /// The requester's tenant, when the call carries one.
    pub(super) requester_tenant: Option<Uuid>,
    /// Whose data the tool body reads.
    pub(super) reader: Uuid,
    /// The reader's tenant, when known.
    pub(super) reader_tenant: Option<Uuid>,
}

impl Parties {
    /// The parties of a call by `user` under `tenant` that reads `subject`
    /// when it resolved one, and the caller's own data otherwise.
    pub(super) fn of(subject: Option<&ReadSubject>, user: Uuid, tenant: Option<Uuid>) -> Self {
        subject.map_or(
            Self {
                requester: user,
                requester_tenant: tenant,
                reader: user,
                reader_tenant: tenant,
            },
            |subject| Self {
                requester: subject.requester,
                requester_tenant: Some(subject.requester_tenant.as_uuid()),
                reader: subject.athlete(),
                reader_tenant: Some(subject.tenant().as_uuid()),
            },
        )
    }
}

impl UniversalExecutor {
    /// Whether `tool_name` is enabled for this call: for the requester under
    /// their tenant, and under the reader's tenant when the call reads a
    /// coached athlete filed elsewhere.
    pub(super) async fn tool_enabled(&self, tool_name: &str, parties: Parties) -> bool {
        if let Some(tenant) = parties.requester_tenant {
            if !guardian::caller_tool_enabled(
                &self.resources,
                TenantId::from_uuid(tenant),
                parties.requester,
                tool_name,
            )
            .await
            {
                return false;
            }
        }
        match parties.reader_tenant {
            Some(reader) if parties.reader_tenant != parties.requester_tenant => {
                guardian::tenant_tool_enabled(
                    &self.resources,
                    TenantId::from_uuid(reader),
                    tool_name,
                )
                .await
            }
            _ => true,
        }
    }

    /// Whose data a call to `tool_name` reads: the caller's own (`None`), or
    /// the coached athlete a training- or wellness-class tool reads for them
    /// — the one this executor's call already reads, for a nested dispatch,
    /// or the one [`resolve_read_subject`] resolves. The inner `Err` is the
    /// in-band refusal the call answers with instead. Such a tool's `athlete`
    /// argument is taken out of `args`: its body reads its context, never the
    /// name. Any other tool keeps its own (the plan and calendar tools).
    pub(super) async fn read_subject(
        &self,
        tool_name: &str,
        args: &mut JsonValue,
        requester: Uuid,
        requester_tenant: Option<Uuid>,
    ) -> Result<Result<Option<ReadSubject>, UniversalResponse>, ProtocolError> {
        let class = self.resources.tool_registry().data_class(tool_name);
        if let Some(inherited) = &self.read_subject {
            // A nested call reads the athlete its caller reads, and only what
            // their scope was resolved for: their wellness only where the
            // caller's read could have read it.
            match class {
                Some(DataClass::Training) => {}
                Some(DataClass::Wellness) => {
                    if let Some(withheld) = inherited.wellness_withheld() {
                        return Ok(Err(read_subject::wellness_class_refusal(
                            tool_name, &withheld,
                        )));
                    }
                }
                _ => return Ok(Err(read_subject::outside_class_refusal(tool_name))),
            }
            take_athlete_argument(args);
            return Ok(Ok(Some(inherited.clone())));
        }
        let Some(class) = class.filter(|class| class.reads_a_coached_athlete()) else {
            return Ok(Ok(None));
        };
        let athlete = take_athlete_argument(args);
        let first_party =
            ai_scope::effective_transport(self.transport).is_some_and(Transport::is_first_party);
        let named_externally = self.caller_writes_reply && !first_party;
        let resolved = resolve_read_subject(ReadSubjectRequest {
            resources: &self.resources,
            requester,
            requester_tenant: requester_tenant.map(TenantId::from_uuid),
            seat: self.seat,
            class,
            conversation_id: self.conversation_id.as_deref(),
            athlete: athlete.as_deref(),
            named_externally,
            tool_name,
        })
        .await
        .map_err(|e| {
            ProtocolError::InternalError(format!("resolving whose data '{tool_name}' reads: {e}"))
        })?;
        if let Ok(Some(subject)) = &resolved {
            info!(
                requester = %subject.requester,
                athlete = %subject.athlete(),
                group_id = %subject.access.group.id,
                tool = %tool_name,
                class = ?class,
                source = subject.via.as_str(),
                "coach reading a coached athlete's data"
            );
        }
        Ok(resolved)
    }

    /// What a read of a coached athlete answers when the athlete's `provider`
    /// connection needs reconnecting: in band, naming the athlete.
    ///
    /// The reconnect is theirs to make, in their own app. Raised as the
    /// re-auth error, it would reach the chat pipeline's reconnect flow,
    /// which mints a hosted login for the turn's user — the coach — and
    /// connects the coach's own account (carnet#1387).
    pub(super) fn athlete_reconnect_refusal(
        &self,
        tool_name: &str,
        subject: &ReadSubject,
        provider: &str,
    ) -> UniversalResponse {
        let brand = backend_resolver::brand_name(self.resources.provider_registry(), provider)
            .unwrap_or(provider);
        read_subject::athlete_reconnect_refusal(tool_name, &subject.access.display_name, brand)
    }
}

/// Take the reconnect offer out of a read of a coached athlete: the backend a
/// window served without one of the athlete's connections names for the chat
/// pipeline's reconnect mint, which would connect the coach's own account.
/// The note to the model stays, so it can say which of the athlete's
/// connections needs them (carnet#1387).
pub(super) fn withhold_reconnect_offer(response: &mut UniversalResponse) {
    if let Some(JsonValue::Object(sidecar)) = response
        .result
        .as_mut()
        .and_then(|result| result.get_mut(RECONNECT_REQUIRED_KEY))
    {
        sidecar.remove(RECONNECT_BACKEND_KEY);
    }
}

/// The context a read of a coached athlete runs under: the athlete and their
/// tenant, never an admin, so the tool body reads the athlete exactly as it
/// reads a caller.
pub(super) fn subject_tool_context(subject: &ReadSubject) -> ToolContext {
    ToolContext::new()
        .with_user(subject.athlete().to_string())
        .with_auth_method(AuthMethod::JwtBearer.as_str())
        .as_admin(false)
        .with_tenant(subject.tenant().to_string())
}

/// Take the `athlete` argument out of `args`: a string as sent, anything
/// else that is not null as its JSON text, which names no athlete and is
/// refused like any other name that matches no one.
fn take_athlete_argument(args: &mut JsonValue) -> Option<String> {
    match args.as_object_mut()?.remove(ATHLETE_ARGUMENT)? {
        JsonValue::Null => None,
        JsonValue::String(name) => Some(name),
        other => Some(other.to_string()),
    }
}

/// The tronc context a dispatch hands its tool: the caller's identity, admin
/// flag and tenant as the request names them.
pub(super) async fn build_tool_context(
    resources: &Arc<dyn ToolRuntime>,
    request: &UniversalRequest,
) -> Result<(ToolContext, bool), ProtocolError> {
    let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

    let is_admin = resources
        .repos()
        .users
        .get_global(user_uuid)
        .await
        .ok()
        .flatten()
        .is_some_and(|user| user.is_admin);

    let mut ctx = ToolContext::new()
        .with_user(user_uuid.to_string())
        .with_auth_method(AuthMethod::JwtBearer.as_str())
        .as_admin(is_admin);

    if let Some(raw) = request.tenant_id.as_deref() {
        let uuid = Uuid::parse_str(raw)
            .map_err(|e| ProtocolError::InvalidRequest(format!("Invalid tenant_id format: {e}")))?;
        ctx = ctx.with_tenant(TenantId::from_uuid(uuid).to_string());
    }

    Ok((ctx, is_admin))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::reconnect::offer_in_payload;

    #[test]
    fn a_coachs_window_keeps_the_note_and_loses_the_mints_backend() {
        let mut response = UniversalResponse {
            success: true,
            result: Some(json!({
                "activity_list": "…",
                "reconnect_required": {
                    "provider": "Garmin",
                    "provider_slug": "sciotte_garmin",
                    "note": "Garmin needs reconnecting."
                }
            })),
            error: None,
            metadata: None,
        };
        assert!(
            offer_in_payload("get_activities", response.result.as_ref().unwrap()).is_some(),
            "the fixture carries an offer"
        );
        withhold_reconnect_offer(&mut response);
        let result = response.result.unwrap();
        assert_eq!(
            offer_in_payload("get_activities", &result),
            None,
            "bug: no reconnect offer survives a read of a coached athlete"
        );
        assert_eq!(
            result["reconnect_required"]["note"], "Garmin needs reconnecting.",
            "bug: the model still reads which connection needs the athlete"
        );
    }

    #[test]
    fn a_result_with_no_offer_is_left_as_it_is() {
        let payload = json!({ "activities": [] });
        let mut response = UniversalResponse {
            success: true,
            result: Some(payload.clone()),
            error: None,
            metadata: None,
        };
        withhold_reconnect_offer(&mut response);
        assert_eq!(
            response.result,
            Some(payload),
            "bug: nothing is taken from a result that offers no reconnect"
        );
    }
}
