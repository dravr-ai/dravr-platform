// ABOUTME: Thin re-export layer delegating to the pierre-auth crate
// ABOUTME: Preserves `crate::key_management::*` paths for the rest of the root crate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_auth::key_management::*;
