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

use std::env;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use embacle::sandbox::{apply_sandbox, SandboxPolicy};
use pierre_pdf_text::{Outcome, Refusal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;

/// The child binary's name.
pub(super) const EXTRACTOR_BIN: &str = "pierre-pdf-text";

/// How long the child may take. A real report reads in well under a second.
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(30);

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
    apply_sandbox(
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

/// The text of the PDF `bytes`, page by page, read by the child `program`.
///
/// # Errors
///
/// The [`ExtractFailure`] that applies.
pub(super) async fn extract(
    program: &Path,
    bytes: Vec<u8>,
) -> Result<Vec<String>, ExtractFailure> {
    let mut child = command(program)
        .spawn()
        .map_err(ExtractFailure::Unavailable)?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let run = async move {
        // The child may stop reading early (a refusal past the size bound),
        // so a broken pipe is not a failure: its exit and answer decide.
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
    };
    // On timeout `run` is dropped with the child it owns: `kill_on_drop`.
    let (answer, status) = timeout(EXTRACTION_TIMEOUT, run)
        .await
        .map_err(|_| ExtractFailure::TimedOut)?;
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
