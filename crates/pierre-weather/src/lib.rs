// ABOUTME: Re-export facade for dravr-meteo weather provider + cache library
// ABOUTME: All types, traits, and provider adapters are canonical in dravr-meteo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Weather Gateway
//!
//! This crate re-exports the [`dravr-meteo`](https://crates.io/crates/dravr-meteo)
//! weather library. All types, traits, and provider adapters live in `dravr-meteo`;
//! this crate exists solely for workspace path compatibility — the audit's
//! `dravr-*` boundary recommendation.

pub use dravr_meteo::*;
