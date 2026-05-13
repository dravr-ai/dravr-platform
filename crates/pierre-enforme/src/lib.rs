// ABOUTME: Re-export facade for dravr-enforme health-data sync library
// ABOUTME: All store traits, models, and provider builders are canonical in dravr-enforme
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Health-Sync Gateway
//!
//! This crate re-exports the [`dravr-enforme`](https://crates.io/crates/dravr-enforme)
//! health-data pull library. All types, traits, and provider builders live in
//! `dravr-enforme`; this crate exists for workspace path compatibility — the
//! audit's `dravr-*` boundary recommendation.

pub use dravr_enforme::*;
