// ABOUTME: The one way the pipeline detaches a task: spawned with a watcher that reports how it ended
// ABOUTME: A panic in detached work is logged with whom the work was for, never lost with its handle

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detached work, supervised.
//!
//! Some of the pipeline's writes outlive the code that asks for them: a tool
//! round written while the turn goes on, a stopped notice settled again
//! when a headless call reports after the stop (carnet#1519). Spawning them
//! and dropping the handle loses a panic with it. [`spawn_supervised`] spawns
//! a second, trivial task that awaits the first and logs how it failed, under
//! the turn or conversation it was for.

use std::future::Future;

use pierre_core::models::ConversationTurnId;
use tokio::runtime::Handle;
use tokio::task::{JoinError, JoinHandle};
use tracing::error;

/// Whom detached work was for, named on the line that reports its failure.
#[derive(Debug, Clone)]
pub(crate) enum TaskOwner {
    /// One chat turn.
    Turn(ConversationTurnId),
    /// One conversation, by its id.
    Conversation(String),
}

/// Spawn `work` on `runtime` with a watcher that logs it at error level when
/// it does not run to its end.
///
/// A panic is what that log line reports in practice: the work's own handle
/// stays with the watcher, so nothing can abort it, and the one other way it
/// is cancelled, the runtime shutting down, takes the watcher with it.
///
/// `task` names the work in the log line. The line leaves out the panic's
/// payload, which can carry text a turn was handling. The returned handle is
/// the watcher's, resolving to whether `work` ran to its end; callers that do
/// not need to know drop it, which leaves the watcher running.
pub(crate) fn spawn_supervised<F>(
    runtime: &Handle,
    task: &'static str,
    owner: TaskOwner,
    work: F,
) -> JoinHandle<bool>
where
    F: Future<Output = ()> + Send + 'static,
{
    let handle = runtime.spawn(work);
    runtime.spawn(async move {
        handle
            .await
            .inspect_err(|failure| report(task, &owner, failure))
            .is_ok()
    })
}

/// Log how detached work failed.
fn report(task: &'static str, owner: &TaskOwner, failure: &JoinError) {
    let panicked = failure.is_panic();
    let task_id = failure.id();
    match owner {
        TaskOwner::Turn(turn_id) => error!(
            task,
            turn_id = %turn_id,
            panicked,
            %task_id,
            "a detached task failed"
        ),
        TaskOwner::Conversation(conversation_id) => error!(
            task,
            conversation_id = conversation_id.as_str(),
            panicked,
            %task_id,
            "a detached task failed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::{Result as IoResult, Write};
    use std::panic;
    use std::sync::Once;

    use tracing::subscriber::set_global_default;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    thread_local! {
        /// What the capturing subscriber wrote on this thread.
        static CAPTURED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    /// Writes into the calling thread's [`CAPTURED`] buffer.
    ///
    /// One global subscriber is installed once for the test binary, so no
    /// test swaps dispatchers under another's feet. `#[tokio::test]` runs on
    /// a current-thread runtime, so the watcher logs on the test's own thread.
    struct ThreadSink;

    impl Write for ThreadSink {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            CAPTURED.with(|captured| captured.borrow_mut().extend_from_slice(buf));
            Ok(buf.len())
        }
        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for ThreadSink {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            Self
        }
    }

    /// Install the capturing subscriber once, and empty this thread's buffer.
    fn capture_logs() {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_writer(|| ThreadSink)
                .with_ansi(false)
                .finish();
            set_global_default(subscriber).expect("no other global subscriber in this binary");
        });
        CAPTURED.with(|captured| captured.borrow_mut().clear());
    }

    /// What the capturing subscriber wrote on this thread since
    /// [`capture_logs`].
    fn captured() -> String {
        CAPTURED.with(|captured| String::from_utf8_lossy(&captured.borrow()).into_owned())
    }

    /// A detached task that panics is reported by its watcher, under the turn
    /// it was for and without its payload, and one that finishes is not: the
    /// watcher's handle resolves to which it was, with no sleep, because the
    /// watcher only ends once the work has.
    #[tokio::test]
    async fn a_panicking_detached_task_is_reported() {
        capture_logs();
        let runtime = Handle::current();
        let turn_id = ConversationTurnId::new();
        let failed = spawn_supervised(&runtime, "test_settle", TaskOwner::Turn(turn_id), async {
            panic::resume_unwind(Box::new("the athlete's private words"))
        });
        assert_eq!(
            failed.await.ok(),
            Some(false),
            "bug: a panic in detached work must reach its watcher"
        );
        let log = captured();
        assert!(
            log.contains("a detached task failed")
                && log.contains("task=\"test_settle\"")
                && log.contains(&format!("turn_id={turn_id}"))
                && log.contains("panicked=true"),
            "bug: the failure is logged naming the task and the turn it was for: {log}"
        );
        assert!(
            !log.contains("private words"),
            "bug: the panic's payload never reaches the log: {log}"
        );

        capture_logs();
        let finished = spawn_supervised(
            &runtime,
            "test_settle",
            TaskOwner::Conversation("conversation-1".to_owned()),
            async {},
        );
        assert_eq!(
            finished.await.ok(),
            Some(true),
            "bug: detached work that finished is not reported as failed"
        );
        let log = captured();
        assert!(
            !log.contains("a detached task failed"),
            "bug: detached work that finished logs no failure: {log}"
        );
    }
}
