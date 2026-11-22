// ABOUTME: Runs the `pierre-pdf-text` child on an uploaded PDF: bytes on stdin, one JSON outcome on stdout, a wall-clock bound
// ABOUTME: A child that crashes, hangs, overruns its memory or answers garbage is a failure here, never a server crash

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The PDF child
//!
//! The server never parses an uploaded PDF in its own process. It spawns
//! [`EXTRACTOR_BIN`] (found beside its own executable unless the context
//! names one: [`beside_server`]), with the environment
//! cleared the way embacle clears a CLI runner's
//! ([`embacle::sandbox::apply_sandbox`], with no key allowed through), writes
//! the bytes to its stdin and reads one [`Outcome`] from its stdout, within
//! [`EXTRACTION_TIMEOUT`]. The child caps its own memory
//! ([`pierre_pdf_text::MEMORY_LIMIT_BYTES`]).
//!
//! embacle's `run_cli_command_with_input` is not used: it logs the child's
//! stdout at trace level, and here that would be the athlete's lab report.
//!
//! At most [`MAX_CONCURRENT_CHILDREN`] child is alive under one server
//! context, the process's, as FIT decodes are bounded by their permits
//! (`pierre_providers::fit_decode`): an instance serves up to 80 requests at
//! once, and a few children at their 512 MiB cap would exhaust its 2 GiB. An
//! upload waits for the permit only as long as its caller's budget allows
//! ([`ExtractFailure::Busy`]).

use std::env;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

use embacle::sandbox::{self, SandboxPolicy};
use pierre_pdf_text::{Outcome, Refusal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Semaphore;
use tokio::time;

/// The child binary's name.
pub(super) const EXTRACTOR_BIN: &str = "pierre-pdf-text";

/// How long the child may take. A real report reads in well under a second,
/// and a decompression bomb ends at its memory cap in a few.
pub(super) const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(15);

/// How many PDF children may be alive at once under one server context. One
/// at its memory cap holds 512 MiB of a 2 GiB instance; a real report is
/// read in well under a second, so a second upload waits only that long.
pub(super) const MAX_CONCURRENT_CHILDREN: usize = 1;

/// The permits a server context's PDF children are started under.
pub(super) fn child_permits() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_CONCURRENT_CHILDREN))
}

/// The most stdout the child's answer may hold: the text bound, JSON-escaped
/// at worst six bytes a byte, plus the envelope. A longer answer is cut, so
/// it does not parse and is refused.
const MAX_ANSWER_BYTES: usize = pierre_pdf_text::MAX_TEXT_BYTES * 6 + 64 * 1024;

/// Why the child gave no pages.
#[derive(Debug)]
pub(super) enum ExtractFailure {
    /// It read the PDF and refused it.
    Refused(Refusal),
    /// It could not be found or started.
    Unavailable(io::Error),
    /// It ran past [`EXTRACTION_TIMEOUT`] and was killed.
    TimedOut,
    /// It ended without an answer: a crash, an abort past its memory cap, a
    /// stack overflow or a panic.
    Ended(ExitStatus),
    /// Its answer did not parse, or ran past [`MAX_ANSWER_BYTES`].
    Malformed,
    /// No child permit came free within the caller's wait: another report
    /// is being read.
    Busy,
}

/// What the log says of a failure: never the PDF's content, which the child's
/// answer is the only carrier of and which is not kept.
impl fmt::Display for ExtractFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => write!(f, "refused: {reason:?}"),
            Self::Unavailable(error) => write!(f, "not started: {error}"),
            Self::TimedOut => write!(f, "killed after {}s", EXTRACTION_TIMEOUT.as_secs()),
            Self::Ended(status) => write!(f, "ended without an answer: {status}"),
            Self::Malformed => f.write_str("answered something that does not parse"),
            Self::Busy => f.write_str("no child permit came free in time"),
        }
    }
}

/// The child beside this executable, where the image and every cargo build
/// of the server put it, or why it is not there.
pub(super) fn beside_server() -> Result<PathBuf, String> {
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let name = format!("{EXTRACTOR_BIN}{}", env::consts::EXE_SUFFIX);
    let candidate = executable
        .parent()
        .map(|directory| directory.join(&name))
        .filter(|candidate| candidate.is_file());
    candidate.ok_or_else(|| format!("{name} is not beside {}", executable.display()))
}

/// The child, ready to start: no environment, working in the temp directory.
fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    sandbox::apply_sandbox(
        &mut command,
        &SandboxPolicy::new(env::temp_dir()).with_env_keys(Vec::new()),
    );
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

/// The text of the PDF `bytes`, page by page, read by the child `program`
/// once one of `permits` is free, waiting for one at most `wait`.
///
/// The permit is held by the child's own task, not by the caller: a caller
/// dropped mid-read (an upload whose client hung up) cannot admit a second
/// child while its own still runs, which ends within [`EXTRACTION_TIMEOUT`].
///
/// # Errors
///
/// The [`ExtractFailure`] that applies: [`ExtractFailure::Busy`] when no
/// permit came free within `wait`.
pub(super) async fn extract(
    (program, permits): (&Path, &Arc<Semaphore>),
    bytes: Vec<u8>,
    wait: Duration,
) -> Result<Vec<String>, ExtractFailure> {
    let permit = match time::timeout(wait, Arc::clone(permits).acquire_owned()).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(closed)) => return Err(ExtractFailure::Unavailable(io::Error::other(closed))),
        Err(_) => return Err(ExtractFailure::Busy),
    };
    let program = program.to_path_buf();
    tokio::spawn(async move {
        let held = permit;
        let read = run_child(&program, bytes).await;
        drop(held);
        read
    })
    .await
    .map_err(|e| ExtractFailure::Unavailable(io::Error::other(e)))?
}

/// The child `program` run on `bytes` to its answer, or killed and reaped
/// past [`EXTRACTION_TIMEOUT`], so no child outlives its permit.
async fn run_child(program: &Path, bytes: Vec<u8>) -> Result<Vec<String>, ExtractFailure> {
    let mut child = command(program)
        .spawn()
        .map_err(ExtractFailure::Unavailable)?;
    let ran = time::timeout(EXTRACTION_TIMEOUT, answer_of(&mut child, bytes)).await;
    let Ok((answer, status)) = ran else {
        // Killed and waited for before the permit is let go.
        child.kill().await.ok();
        return Err(ExtractFailure::TimedOut);
    };
    let status = status.map_err(ExtractFailure::Unavailable)?;
    if !status.success() {
        return Err(ExtractFailure::Ended(status));
    }
    if answer.len() > MAX_ANSWER_BYTES {
        return Err(ExtractFailure::Malformed);
    }
    match serde_json::from_slice::<Outcome>(&answer) {
        Ok(Outcome::Pages { pages }) => Ok(pages),
        Ok(Outcome::Refused { reason }) => Err(ExtractFailure::Refused(reason)),
        Err(_) => Err(ExtractFailure::Malformed),
    }
}

/// Feed `bytes` to `child`, read its answer, and wait for it to end.
async fn answer_of(child: &mut Child, bytes: Vec<u8>) -> (Vec<u8>, io::Result<ExitStatus>) {
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    // The child may stop reading early (a refusal past the size bound), so a
    // broken pipe is not a failure: its exit and answer decide.
    let feed = async move {
        if let Some(mut stdin) = stdin {
            if stdin.write_all(&bytes).await.is_ok() {
                stdin.shutdown().await.ok();
            }
        }
    };
    let read = async move {
        let mut answer = Vec::new();
        if let Some(stdout) = stdout {
            let cap = u64::try_from(MAX_ANSWER_BYTES).unwrap_or(u64::MAX);
            stdout.take(cap + 1).read_to_end(&mut answer).await.ok();
        }
        answer
    };
    let ((), answer, status) = tokio::join!(feed, read, child.wait());
    (answer, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_upload_that_cannot_get_the_permit_in_its_wait_is_busy() {
        let permits = child_permits();
        let held = Arc::clone(&permits)
            .acquire_owned()
            .await
            .expect("bug: the child permits are closed");
        let waited = extract(
            (Path::new("/nonexistent"), &permits),
            Vec::new(),
            Duration::from_millis(20),
        )
        .await;
        drop(held);
        assert!(
            matches!(waited, Err(ExtractFailure::Busy)),
            "bug: an upload past its wait was not answered busy: {waited:?}"
        );
    }

    /// A child that runs on after its upload is dropped keeps the permit, so
    /// no second child starts beside it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dropped_upload_holds_its_permit_until_its_child_ends() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("bug: no temp dir");
        let program = dir.path().join("slow-pdf-text");
        fs::write(&program, "#!/bin/sh\nexec /bin/sleep 2\n").expect("bug: script not written");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755))
            .expect("bug: script not executable");
        let permits = child_permits();
        let upload = tokio::spawn({
            let permits = Arc::clone(&permits);
            async move { extract((program.as_path(), &permits), Vec::new(), Duration::ZERO).await }
        });
        time::timeout(Duration::from_secs(5), async {
            while permits.available_permits() == MAX_CONCURRENT_CHILDREN {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("bug: the child never took its permit");
        upload.abort();
        assert!(
            upload.await.is_err_and(|e| e.is_cancelled()),
            "bug: the upload was not dropped mid-read"
        );
        assert_eq!(
            permits.available_permits(),
            0,
            "bug: a dropped upload let go of the permit while its child runs"
        );
        time::timeout(Duration::from_secs(10), async {
            while permits.available_permits() < MAX_CONCURRENT_CHILDREN {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("bug: the permit did not come back once the child ended");
    }
}
