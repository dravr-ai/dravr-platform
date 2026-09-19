// ABOUTME: Repository trait for the A2A reaper's one statement — fail every task a dead instance left non-terminal
// ABOUTME: Written once here; each backend shell supplies its clock, its JSON cast and its staleness predicate as literals

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// The A2A reaper's write.
///
/// A task runs detached from the request that submitted it, so an instance
/// killed mid-run leaves the row non-terminal forever: nothing else writes
/// it, `GetTask` keeps reporting it as running, and a subscriber waits on a
/// channel nobody will close. The reaper calls this on a period; the
/// returned ids let it close those channels and deliver the terminal push.
/// It is its own trait rather than a method on `A2ARepository` because that
/// pair still carries its SQL twice and this one statement is written once.
#[async_trait]
pub trait A2ATaskReaperRepository: Send + Sync {
    /// Fail every task still `submitted` or `working` whose status has not
    /// moved for `stale_after_secs`, stamping `status_message`, and return
    /// the ids it failed.
    async fn fail_stale_tasks(
        &self,
        stale_after_secs: i64,
        status_message: &Value,
    ) -> AppResult<Vec<String>>;
}

/// The bulk fail, assembled from the three things the backends spell
/// differently: the clock expression that stamps `updated_at`, the cast the
/// JSON `status_message` column needs on the bound string, and the predicate
/// that reads a task as stale. `$1` is the message JSON, `$2` the staleness
/// threshold in seconds.
macro_rules! fail_stale_tasks_sql {
    ($now:literal, $message:literal, $stale:literal) => {
        concat!(
            "UPDATE a2a_tasks SET status = 'failed', status_message = ",
            $message,
            ", updated_at = ",
            $now,
            " WHERE status IN ('submitted', 'working') AND ",
            $stale,
            " RETURNING task_id"
        )
    };
}
pub(crate) use fail_stale_tasks_sql;

pub(crate) fn task_id_from_row<R>(row: &R) -> AppResult<String>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.try_get::<String, _>("task_id")
        .map_err(|e| AppError::database(format!("a2a_tasks task_id: {e}")))
}

/// Emit the [`A2ATaskReaperRepository`] implementation for one backend
/// type, given the statement [`fail_stale_tasks_sql!`] assembled with that
/// backend's three literals.
///
/// The body names its helpers and types unqualified, so the invoking shell
/// must `use` every one of them.
macro_rules! impl_a2a_task_reaper_repository {
    ($ty:ty, $sql:expr) => {
        #[async_trait::async_trait]
        impl A2ATaskReaperRepository for $ty {
            async fn fail_stale_tasks(
                &self,
                stale_after_secs: i64,
                status_message: &Value,
            ) -> AppResult<Vec<String>> {
                let status_message_json = serde_json::to_string(status_message)?;
                let rows = sqlx::query($sql)
                    .bind(status_message_json)
                    .bind(stale_after_secs.max(0))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fail stale A2A tasks: {e}"))
                    })?;
                rows.iter().map(task_id_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_a2a_task_reaper_repository;
