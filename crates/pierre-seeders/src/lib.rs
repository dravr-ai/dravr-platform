// ABOUTME: Seeder modules for populating Pierre MCP Server with reference and demo data
// ABOUTME: Each submodule exposes a `run` entry point invoked by `pierre-cli seed <domain>`
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Seeders
//!
//! Reference + demo data seeders invoked by `pierre-cli seed <domain>`. Each
//! submodule owns one domain (admin bootstrap, system coaches, demo users +
//! synthetic activities, mobility catalogue, social graph, LLM-usage samples,
//! insight samples) and exposes a `run` function the CLI dispatches to.

pub mod bootstrap;
pub mod coaches;
pub mod demo_data;
pub mod insight_samples;
pub mod llm_usage;
pub mod mobility;
pub mod social;
pub mod synthetic_activities;
