// ABOUTME: What of an exporter's conversations their data export leaves out: what their own screen withholds
// ABOUTME: Chat rows and summaries about a member the exporter may no longer read, by the one transcript rule

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A coach's conversation can hold rows about a group member: a tool round
//! that read them, the reply drawn from it. The transcript route serves such
//! a row only while the reader may read that member now, with every platform
//! row after it in its topic (carnet#1422). The export is one more read of
//! the transcript for a person, so it applies the same rule
//! ([`transcript_reading::withheld_rows`]), to `chat_messages` and to the
//! summaries in `compaction_blocks`: what a member stopped sharing leaves
//! the coach's screen and their copy of their own data alike (carnet#1511
//! design call 2).
//!
//! Two more tables copy what a conversation held (design call 7):
//!
//! - **`messaging_messages`**: an outbound row a reply was posted as names
//!   that reply (`chat_message_id`) and is left out with it, or when the
//!   exporter no longer reads it in any conversation. What the exporter sent
//!   (inbound) is theirs. An outbound row naming no reply (a command's
//!   answer, a notice, a one-time code, an apology) follows its session's
//!   conversation (design call 10): kept while that conversation withholds
//!   nothing from the exporter, left out once it withholds any row, and left
//!   out when the session cannot be read. A row whose conversation is gone
//!   (the exporter deleted it: the session then names none, and the reply
//!   it copied is in no conversation) is kept when the exporter had no
//!   member's data within reach by the row's `created_at`
//!   (`has_member_history`), so a plain user's export keeps it, and
//!   left out otherwise. The session names its current
//!   conversation, which a self-healed session may have repointed since the
//!   row was sent.
//! - **`a2a_tasks`**: a tool task records the reads its call made, each with
//!   how it was made (`about_members`, carnet#1511 call 19), and is left out
//!   while the exporter may not read one of them
//!   (the one task rule, `pierre_services::task_reading`, which A2A
//!   `GetTask` and `ListTasks` ask too). A coaching text
//!   turn's task is decided by its conversation (`context_id`): left out
//!   when a row the task produced or followed is withheld, which the rows
//!   tell only by when they were stored: any withheld row stored at or
//!   before the task's last update (`updated_at`). A row stored after it
//!   belongs to a later turn. An instant that cannot be read leaves the task
//!   out.
//!
//!   A task from before the record (`about_members` NULL) is kept (design
//!   §6.2) when it named no person and its tool reads its caller's own data
//!   alone, or when the exporter had no member's data within reach by the
//!   task's last update (`has_member_history`); otherwise a coaching text
//!   turn's task follows its thread as above, and any other (a tool call,
//!   even one filed under a thread's id) is left out.
//!
//! - **`group_transcript_entries`**: an entry is exported to its author
//!   while the one transcript rule lets them read it now, as the room page
//!   does: a reply that drew on a member who stopped sharing leaves the
//!   export with the screen (carnet#1511 A6).
//!
//! A row left out this way is not a failure: its table is not named among
//! the failed ones, which are left for a rule that could not be applied. It
//! is named among the withheld ones instead, and the document says it is
//! not complete (design §6.4).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::RoomEntryBody;
use pierre_core::models::TenantId;
use pierre_database::repositories::UserExportTable;
use pierre_database::RepositoryRegistry;
use pierre_services::member_reads;
use pierre_services::task_reading::{self, StoredTask, TaskReading};
use pierre_services::transcript_reading::{self, WithheldRows};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

/// The table of a conversation's rows.
const MESSAGES: &str = "chat_messages";
/// The table of a conversation's summaries.
const SUMMARIES: &str = "compaction_blocks";
/// The table of the messages a messaging channel carried.
const CHANNEL_MESSAGES: &str = "messaging_messages";
/// The table of A2A tasks.
const TASKS: &str = "a2a_tasks";
/// The table of a group's room entries.
const ROOM_ENTRIES: &str = "group_transcript_entries";

/// Whether the export leaves a row out.
#[async_trait]
pub(super) trait RowWithholding: Send + Sync {
    /// Whether `row` of `table` is left out. An error leaves it out too, and
    /// names its table among the failed: the export never writes a row the
    /// rule could not decide.
    async fn withholds(&self, table: &UserExportTable, row: &Value) -> AppResult<bool>;
}

/// The transcript rule for one exporter, decided once per conversation.
pub(super) struct ReaderTranscripts {
    repos: Arc<RepositoryRegistry>,
    reader: Uuid,
    /// The registered tools that read their caller's own data alone.
    own_data_tools: Arc<BTreeSet<String>>,
    tenants: Vec<TenantId>,
    decided: Mutex<HashMap<String, Arc<WithheldRows>>>,
    /// The reads of each group's room the reader may read, decided once.
    room_reads: Mutex<HashMap<Uuid, Arc<Vec<String>>>>,
}

impl ReaderTranscripts {
    /// The rule for `reader`, whose conversations live in `tenants`.
    pub(super) fn new(
        repos: Arc<RepositoryRegistry>,
        reader: Uuid,
        tenants: Vec<TenantId>,
        own_data_tools: Arc<BTreeSet<String>>,
    ) -> Self {
        Self {
            repos,
            reader,
            own_data_tools,
            tenants,
            decided: Mutex::new(HashMap::new()),
            room_reads: Mutex::new(HashMap::new()),
        }
    }

    /// What `conversation_id` withholds from the reader, read under the
    /// row's own tenant when it names one, else the first of the reader's
    /// tenants that holds the conversation for them.
    async fn of(
        &self,
        conversation_id: &str,
        row_tenant: Option<&str>,
    ) -> AppResult<Arc<WithheldRows>> {
        self.held(conversation_id, row_tenant)
            .await?
            .ok_or_else(|| AppError::not_found("Conversation"))
    }

    /// [`Self::of`], `None` when no tenant of the reader's holds the
    /// conversation for them.
    async fn held(
        &self,
        conversation_id: &str,
        row_tenant: Option<&str>,
    ) -> AppResult<Option<Arc<WithheldRows>>> {
        if let Some(known) = self.decided.lock().await.get(conversation_id) {
            return Ok(Some(Arc::clone(known)));
        }
        let named = row_tenant.and_then(|tenant| TenantId::parse_str(tenant).ok());
        let reader = self.reader.to_string();
        let tenants = named.iter().chain(self.tenants.iter());
        for tenant in tenants {
            let Some(conversation) = self
                .repos
                .chat
                .get_conversation(conversation_id, &reader, *tenant)
                .await?
            else {
                continue;
            };
            let withheld = Arc::new(
                transcript_reading::withheld_rows(&self.repos, &conversation, self.reader, *tenant)
                    .await?,
            );
            self.decided
                .lock()
                .await
                .insert(conversation_id.to_owned(), Arc::clone(&withheld));
            return Ok(Some(withheld));
        }
        Ok(None)
    }

    /// Whether a channel row whose conversation no longer exists (the reader
    /// deleted it, or a self-healed session points nowhere) is left out: kept
    /// when the reader had no member's data within reach by the row's
    /// `created_at` (`has_member_history`, the rule a legacy task is kept
    /// by), since nothing in it can be anyone else's; left out otherwise,
    /// and when the instant cannot be read.
    async fn withholds_orphaned_copy(&self, row: &Value) -> AppResult<bool> {
        let Some(at) = row["created_at"]
            .as_str()
            .and_then(|at| Timestamp::parse_from_rfc3339(at).ok())
        else {
            return Ok(true);
        };
        self.repos
            .user_export
            .has_member_history(self.reader, at)
            .await
    }

    /// Whether the outbound channel row `row`, copying the reply `reply_id`,
    /// is left out: with the reply, or, when no conversation the reader reads
    /// holds it any more, unless the reader never had a member's data within
    /// reach ([`Self::withholds_orphaned_copy`]).
    async fn withholds_reply_copy(
        &self,
        row: &Value,
        reply_id: &str,
        row_tenant: Option<&str>,
    ) -> AppResult<bool> {
        let named = row_tenant.and_then(|tenant| TenantId::parse_str(tenant).ok());
        let reader = self.reader.to_string();
        let tenants = named.iter().chain(self.tenants.iter());
        for tenant in tenants {
            let Some(conversation) = self
                .repos
                .chat
                .message_conversation(reply_id, &reader, *tenant)
                .await?
            else {
                continue;
            };
            let tenant = tenant.to_string();
            return Ok(self
                .held(&conversation, Some(&tenant))
                .await?
                .is_none_or(|withheld| withheld.message(reply_id)));
        }
        self.withholds_orphaned_copy(row).await
    }

    /// Whether the A2A task `row` is left out: the one task rule
    /// ([`task_reading::task_reading`]), a text turn's thread read here.
    async fn withholds_task(&self, row: &Value) -> AppResult<bool> {
        let input = task_input(row);
        let task = StoredTask {
            about_members: row["about_members"].as_str(),
            updated_at: row["updated_at"]
                .as_str()
                .and_then(|at| Timestamp::parse_from_rfc3339(at).ok()),
            input: input.as_ref(),
            context_id: row["context_id"].as_str(),
        };
        let own = |tool: &str| self.own_data_tools.contains(tool);
        match task_reading::task_reading(&self.repos, self.reader, &task, own).await? {
            TaskReading::Kept => Ok(false),
            TaskReading::Withheld => Ok(true),
            TaskReading::ByThread { context_id, ended } => Ok(self
                .held(&context_id, None)
                .await?
                .is_none_or(|withheld| withheld.withheld_through(ended))),
        }
    }

    /// Whether the outbound channel row `row`, which names no reply, is left
    /// out: kept only while its session's conversation withholds nothing
    /// from the reader.
    async fn withholds_session_copy(&self, row: &Value) -> AppResult<bool> {
        let Some(tenant) = row["tenant_id"]
            .as_str()
            .and_then(|tenant| TenantId::parse_str(tenant).ok())
        else {
            return Ok(true);
        };
        let Some(session) = self
            .repos
            .messaging
            .get_session(tenant, text(row, "session_id")?)
            .await?
        else {
            return Ok(true);
        };
        let Some(conversation) = session["pierre_conversation_id"].as_str() else {
            return self.withholds_orphaned_copy(row).await;
        };
        let tenant = tenant.to_string();
        match self.held(conversation, Some(&tenant)).await? {
            Some(withheld) => Ok(withheld.any()),
            None => self.withholds_orphaned_copy(row).await,
        }
    }
}

/// An A2A task's input message, stored as JSON text on one backend and a
/// document on the other; `None` when it cannot be read.
fn task_input(row: &Value) -> Option<Value> {
    match &row["parameters"] {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        other => Some(other.clone()),
    }
}

/// `row`'s text column `column`.
fn text<'row>(row: &'row Value, column: &str) -> AppResult<&'row str> {
    row[column]
        .as_str()
        .ok_or_else(|| AppError::internal(format!("export row without its {column}")))
}

impl ReaderTranscripts {
    /// The reads of `group`'s room the reader may read now, by the one rule
    /// (carnet#1511 call 19), decided once per export.
    async fn room_reads(&self, group: Uuid) -> AppResult<Arc<Vec<String>>> {
        if let Some(known) = self.room_reads.lock().await.get(&group) {
            return Ok(Arc::clone(known));
        }
        let readable = Arc::new(
            member_reads::readable_room_keys(
                &self.repos,
                member_reads::Reader::Person(self.reader),
                &[group],
            )
            .await?,
        );
        self.room_reads
            .lock()
            .await
            .insert(group, Arc::clone(&readable));
        Ok(readable)
    }

    /// Whether a room entry the exporter is the author of is left out: the
    /// one transcript rule withholds it from them now, as the room page does
    /// (a reply that drew on a member who stopped sharing, an entry with no
    /// record of what it drew on is kept, since its author reads it).
    async fn withholds_room_entry(&self, row: &Value) -> AppResult<bool> {
        let entry = Uuid::parse_str(text(row, "id")?)
            .map_err(|e| AppError::internal(format!("export room entry id: {e}")))?;
        let tenant = TenantId::parse_str(text(row, "tenant_id")?)
            .map_err(|e| AppError::internal(format!("export room entry tenant: {e}")))?;
        let group = text(row, "group_id")?;
        let group_id = Uuid::parse_str(group)
            .map_err(|e| AppError::internal(format!("export room entry group: {e}")))?;
        let readable = self.room_reads(group_id).await?;
        let read = self
            .repos
            .groups
            .get_room_transcript_entry_for(group, self.reader, entry, [tenant, tenant], &readable)
            .await?;
        Ok(!matches!(
            read.map(|entry| entry.body),
            Some(RoomEntryBody::Shared(_))
        ))
    }
}

#[async_trait]
impl RowWithholding for ReaderTranscripts {
    async fn withholds(&self, table: &UserExportTable, row: &Value) -> AppResult<bool> {
        match table.name {
            MESSAGES => {
                let withheld = self.of(text(row, "conversation_id")?, None).await?;
                Ok(withheld.message(text(row, "id")?))
            }
            SUMMARIES => {
                let withheld = self
                    .of(text(row, "conversation_id")?, row["tenant_id"].as_str())
                    .await?;
                let about = row["about_user_id"]
                    .as_str()
                    .map(|member| {
                        Uuid::parse_str(member).map_err(|e| {
                            AppError::internal(format!("export summary about_user_id: {e}"))
                        })
                    })
                    .transpose()?;
                Ok(withheld.summary(
                    text(row, "first_message_id")?,
                    text(row, "last_message_id")?,
                    about,
                ))
            }
            CHANNEL_MESSAGES => {
                if row["direction"].as_str() == Some("inbound") {
                    return Ok(false);
                }
                match row["chat_message_id"].as_str() {
                    Some(reply) => {
                        self.withholds_reply_copy(row, reply, row["tenant_id"].as_str())
                            .await
                    }
                    None => self.withholds_session_copy(row).await,
                }
            }
            TASKS => self.withholds_task(row).await,
            ROOM_ENTRIES => self.withholds_room_entry(row).await,
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use async_trait::async_trait;
    use pierre_core::errors::AppResult;
    use pierre_database::repositories::UserExportTable;
    use serde_json::Value;

    use super::RowWithholding;

    /// The fake tables of the export's unit tests hold no conversation
    /// about anyone.
    pub(crate) struct WithholdsNothing;

    #[async_trait]
    impl RowWithholding for WithholdsNothing {
        async fn withholds(&self, _table: &UserExportTable, _row: &Value) -> AppResult<bool> {
            Ok(false)
        }
    }
}
