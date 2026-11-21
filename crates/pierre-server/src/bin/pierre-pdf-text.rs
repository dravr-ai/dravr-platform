// ABOUTME: The child process an uploaded lab report's text is read in: a PDF on stdin, one JSON outcome on stdout
// ABOUTME: Its allocator is capped, so a PDF that asks for more memory ends this process, never the server

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-pdf-text`: the server spawns it per uploaded lab report
//! (`services::lab_documents`), with a wall-clock timeout and a cleared
//! environment. See [`pierre_pdf_text`] for the protocol.

use std::alloc::System;
use std::io;
use std::process::ExitCode;

use cap::Cap;

/// Every allocation this process makes, refused past
/// [`pierre_pdf_text::MEMORY_LIMIT_BYTES`]: a refused allocation aborts it.
#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

fn main() -> ExitCode {
    if ALLOCATOR
        .set_limit(pierre_pdf_text::MEMORY_LIMIT_BYTES)
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    pierre_pdf_text::run(io::stdin().lock(), io::stdout().lock())
}
