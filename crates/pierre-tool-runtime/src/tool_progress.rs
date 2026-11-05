// ABOUTME: Reports a tool call to the turn's sink as a step: a started frame, then a closing one
// ABOUTME: One vocabulary for every loop, so a client draws one trail whichever loop ran the call

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reports a tool call to the turn's sink as a step, whichever loop ran it.
//!
//! A turn the athlete watches sees each tool call as a step, in the ACP words
//! the Copilot turn reports its own calls in. A call the Guardian blocked or
//! parked never ran, and its closing frame says so ([`TOOL_STATUS_NOT_RUN`])
//! instead of checking it off.

use pierre_services::chat_stream::{
    TurnEvent, TurnEventSink, TurnProgress, TOOL_STATUS_COMPLETED, TOOL_STATUS_FAILED,
    TOOL_STATUS_NOT_RUN,
};
use tracing::trace;

use crate::protocol::types::{guardian_block_reason, UniversalResponse};

/// Send one event, tracing a receiver that has gone away.
///
/// A send fails when the receiver was dropped — the client disconnected or the
/// pipeline aborted — and the drain carries on regardless so the run still
/// completes; the event's content stays out of the log.
pub fn deliver(sink: &TurnEventSink, event: TurnEvent) {
    if sink.send(event).is_err() {
        trace!("turn stream receiver dropped; event not delivered");
    }
}

/// The status a call's closing progress frame reports: it ran and succeeded,
/// it ran and failed, or the Guardian stopped it before it ran.
#[must_use]
pub(crate) fn closing_status(tool_response: &UniversalResponse) -> &'static str {
    if guardian_block_reason(tool_response).is_some() {
        TOOL_STATUS_NOT_RUN
    } else if tool_response.success {
        TOOL_STATUS_COMPLETED
    } else {
        TOOL_STATUS_FAILED
    }
}

/// Report call `call_id` of `tool_name` in `status` to the turn's sink, when
/// the turn has one. The title is the tool's name: the client maps it to the
/// step's words, never shows it raw.
pub(crate) fn report_call(
    sink: Option<&TurnEventSink>,
    call_id: &str,
    tool_name: &str,
    status: &str,
) {
    if let Some(sink) = sink {
        deliver(
            sink,
            TurnEvent::Progress(TurnProgress::tool(
                call_id.to_owned(),
                tool_name.to_owned(),
                status.to_owned(),
            )),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn response(success: bool, blocked_reason: Option<&str>) -> UniversalResponse {
        UniversalResponse {
            success,
            result: None,
            error: (!success).then(|| "refused".to_owned()),
            metadata: blocked_reason.map(|reason| {
                HashMap::from([(
                    "blocked_reason".to_owned(),
                    serde_json::Value::String(reason.to_owned()),
                )])
            }),
        }
    }

    #[test]
    fn a_call_that_ran_closes_completed_or_failed() {
        assert_eq!(
            closing_status(&response(true, None)),
            TOOL_STATUS_COMPLETED,
            "a successful call is a step done"
        );
        assert_eq!(
            closing_status(&response(false, None)),
            TOOL_STATUS_FAILED,
            "a call that ran and failed is still a step taken"
        );
    }

    #[test]
    fn a_call_the_guardian_stopped_closes_as_never_run() {
        assert_eq!(
            closing_status(&response(false, Some("guardian"))),
            TOOL_STATUS_NOT_RUN,
            "a denied call never ran"
        );
        assert_eq!(
            closing_status(&response(false, Some("guardian_confirm"))),
            TOOL_STATUS_NOT_RUN,
            "a call parked for confirmation has not run yet"
        );
    }

    #[test]
    fn a_block_stamp_on_a_success_or_an_unknown_reason_is_not_a_block() {
        assert_eq!(
            closing_status(&response(true, Some("guardian"))),
            TOOL_STATUS_COMPLETED,
            "a call that succeeded ran, whatever its metadata says"
        );
        assert_eq!(
            closing_status(&response(false, Some("rate_limit"))),
            TOOL_STATUS_FAILED,
            "only the Guardian's own stamps mean the call never ran"
        );
    }
}
