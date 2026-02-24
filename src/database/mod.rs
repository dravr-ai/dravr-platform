// ABOUTME: Thin re-export layer delegating to the pierre-database crate
// ABOUTME: Preserves `crate::database::*` paths for the rest of the root crate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_database::database::*;
