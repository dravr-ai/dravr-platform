// ABOUTME: Unit tests for the chat pipeline's hook traits — the per-surface side-effect seams
// ABOUTME: Lives in tests/ because pierre-server forbids #[cfg(test)] inside src/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

// Surface capabilities (what a surface renders, how long a reply may be, which
// model policy applies) are pinned in `surface_profile_test.rs`. What is left
// here is the other half of per-surface behaviour: the hooks a caller installs
// for side effects the pipeline itself must not know about.
use pierre_chat_pipeline::hooks::{IdentityPostProcess, PipelineHooks, ResponsePostProcess};

#[test]
fn identity_post_process_returns_input_unchanged() {
    let p = IdentityPostProcess;
    assert_eq!(p.transform("hello"), "hello");
    assert_eq!(p.transform(""), "");
}

#[test]
fn hooks_none_is_all_none() {
    let hooks = PipelineHooks::none();
    assert!(hooks.response_post_process.is_none());
    assert!(hooks.agui.is_none());
    assert!(hooks.stream_sink.is_none());
    assert!(hooks.scene_publisher.is_none());
}
