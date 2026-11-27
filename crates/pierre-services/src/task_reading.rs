// ABOUTME: Whether a stored A2A task is served to its owner now: the reads it recorded, decided by the one read rule
// ABOUTME: Shared by the data export and A2A GetTask, ListTasks, SubscribeToTask and CancelTask, so a task is withheld alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Reading a stored A2A task
//!
//! A task keeps what its call produced: a tool call's result as its
//! artifact, a coaching text turn's reply. Both can hold another member's
//! data, so every read of a stored task for a person (the data export, and
//! `GetTask`, `ListTasks`, `SubscribeToTask` and `CancelTask` on the A2A
//! surface) asks one
//! answer, [`task_reading`] (carnet#1511 calls 2 and 7):
//!
//! - **A task that recorded its reads** (`about_members`, each read with how
//!   it was made, call 19) is served while the reader may read every one of
//!   them now, by the one rule ([`member_reads::readable_keys`]); a record
//!   that cannot be read withholds it.
//! - **A task from before the record** (`about_members` NULL) is served when
//!   it named no person and its tool reads its caller's own data alone, or
//!   when the reader had no member's data within reach by the task's last
//!   update (`has_member_history`); otherwise a coaching text turn's task
//!   follows its thread ([`TaskReading::ByThread`]): withheld when a row of
//!   its conversation stored at or before its last update is withheld from
//!   the reader. Any other task (a tool call, even one filed under a
//!   thread's id) is withheld.
//!
//! - **A task an admin's support session made** (a [`SupportStamp`]) holds
//!   the data of the user the session acted as, which the admin may read only
//!   while that session lives: once it ends or expires the task is withheld
//!   (carnet#1511 call 28).
//!
//! [`member_reads::readable_keys`]: crate::member_reads::readable_keys

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::errors::AppResult;
use pierre_core::models::CarriedRead;
use pierre_core::permissions::impersonation::SESSION_MINUTES;
use pierre_database::RepositoryRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::transcript_reading;

/// The arguments a call names another person by: a coached athlete, or a
/// group member.
const PERSON_ARGUMENTS: [&str; 4] = ["athlete", "member", "member_id", "athlete_user_id"];

/// A stored task, as its reader's rule reads it.
#[derive(Debug, Clone, Copy)]
pub struct StoredTask<'task> {
    /// The reads its call recorded (`about_members`), `None` for a task from
    /// before the record.
    pub about_members: Option<&'task str>,
    /// When it was last updated; `None` when the instant cannot be read.
    pub updated_at: Option<Timestamp>,
    /// Its input message; `None` when it cannot be read.
    pub input: Option<&'task Value>,
    /// The conversation a coaching text turn ran in (`contextId`).
    pub context_id: Option<&'task str>,
}

/// The record of a task an admin's support session made.
///
/// The session, and the reads its call made as the user. Stored in
/// `about_members` in place of the bare reads, which a build that predates
/// it reads as undecidable and withholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportStamp {
    /// The support session the call ran in.
    pub support_session: String,
    /// The reads its call made.
    pub reads: Vec<CarriedRead>,
}

/// Whether the support session `session_id` is live for `reader`: started by
/// them, not ended, and within its hour.
async fn support_session_live(
    repos: &RepositoryRegistry,
    reader: Uuid,
    session_id: &str,
) -> AppResult<bool> {
    Ok(repos
        .impersonation
        .get_session(session_id)
        .await?
        .is_some_and(|session| {
            session.is_active
                && session.ended_at.is_none()
                && session.impersonator_id == reader
                && Timestamp::now() < session.started_at + Elapsed::minutes(SESSION_MINUTES)
        }))
}

/// Whether a reader is served a stored task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskReading {
    /// Served.
    Kept,
    /// Withheld: it holds a read the reader may not read now, or what it
    /// holds cannot be decided.
    Withheld,
    /// A coaching text turn's task from before the record: withheld when a
    /// row of the conversation `context_id` stored at or before `ended` is
    /// withheld from the reader (`WithheldRows::withheld_through`), which
    /// the caller reads in the tenants it knows the reader by.
    ByThread {
        /// The conversation the turn ran in.
        context_id: String,
        /// The task's last update.
        ended: Timestamp,
    },
}

/// Whether `reader` is served `task` now (module docs). `own_data_tool`
/// answers whether a registered tool reads its caller's own data alone.
///
/// # Errors
///
/// Returns the repository error from the rule's lookups: the caller then
/// serves nothing rather than everything.
pub async fn task_reading(
    repos: &RepositoryRegistry,
    reader: Uuid,
    task: &StoredTask<'_>,
    own_data_tool: impl Fn(&str) -> bool,
) -> AppResult<TaskReading> {
    if let Some(stamp) = task.about_members {
        // A stamp that names no reads cannot be decided, so it is withheld,
        // as an unstamped tool task is.
        let reads = match serde_json::from_str::<Vec<CarriedRead>>(stamp) {
            Ok(reads) => reads,
            Err(_) => match serde_json::from_str::<SupportStamp>(stamp) {
                Ok(support) => {
                    if !support_session_live(repos, reader, &support.support_session).await? {
                        return Ok(TaskReading::Withheld);
                    }
                    support.reads
                }
                Err(_) => return Ok(TaskReading::Withheld),
            },
        };
        return Ok(
            if transcript_reading::reads_withheld(repos, reader, &reads).await? {
                TaskReading::Withheld
            } else {
                TaskReading::Kept
            },
        );
    }
    let Some(ended) = task.updated_at else {
        return Ok(TaskReading::Withheld);
    };
    if task
        .input
        .is_some_and(|input| reads_only_its_caller(input, &own_data_tool))
    {
        return Ok(TaskReading::Kept);
    }
    if !repos.user_export.has_member_history(reader, ended).await? {
        return Ok(TaskReading::Kept);
    }
    // Only a coaching text turn is decided by its thread. A tool call filed
    // under a thread's id (a client may name any) holds its result, which no
    // row of the thread speaks for.
    match task.context_id {
        Some(context) if task.input.is_some_and(|input| !carries_tool_intent(input)) => {
            Ok(TaskReading::ByThread {
                context_id: context.to_owned(),
                ended,
            })
        }
        _ => Ok(TaskReading::Withheld),
    }
}

/// Whether a task's input is a tool call holding only its caller's own data
/// (design §6.2 rule 1): every part asks for a tool that reads no one else,
/// naming no person, and at least one does. A text turn is not.
fn reads_only_its_caller(input: &Value, own_data_tool: &impl Fn(&str) -> bool) -> bool {
    let Some(parts) = input["parts"].as_array() else {
        return false;
    };
    let calls: Vec<&Value> = parts
        .iter()
        .map(|part| &part["data"])
        .filter(|data| !data.is_null())
        .collect();
    !calls.is_empty()
        && calls.iter().all(|call| {
            call["tool_name"].as_str().is_some_and(own_data_tool)
                && PERSON_ARGUMENTS
                    .iter()
                    .all(|argument| call["parameters"][*argument].is_null())
        })
}

/// Whether a task's input asks for a tool: a part carrying `data.tool_name`.
/// An input with no parts counts as one.
fn carries_tool_intent(input: &Value) -> bool {
    input["parts"].as_array().is_none_or(|parts| {
        parts
            .iter()
            .any(|part| !part["data"]["tool_name"].is_null())
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_tool_call_naming_no_one_through_an_own_data_tool_reads_its_caller_alone() {
        let own = |tool: &str| tool == "get_activities";
        let call =
            json!({ "parts": [{ "data": { "tool_name": "get_activities", "parameters": {} } }] });
        assert!(
            reads_only_its_caller(&call, &own),
            "an own-data tool call naming no one reads its caller alone"
        );
        let naming = json!({ "parts": [{ "data": {
            "tool_name": "get_activities", "parameters": { "athlete": "Alex" } } }] });
        assert!(
            !reads_only_its_caller(&naming, &own),
            "a call naming a person"
        );
        let other = json!({ "parts": [{ "data": { "tool_name": "get_roster_overview" } }] });
        assert!(
            !reads_only_its_caller(&other, &own),
            "a tool reading others"
        );
        let text = json!({ "parts": [{ "text": "how was my week?" }] });
        assert!(!reads_only_its_caller(&text, &own), "a text turn");
        assert!(
            !carries_tool_intent(&text),
            "a text-only turn asks for no tool"
        );
        assert!(
            carries_tool_intent(&call),
            "a part carrying a tool name asks for a tool"
        );
        assert!(
            carries_tool_intent(&json!({})),
            "no parts reads as a tool call"
        );
    }
}
