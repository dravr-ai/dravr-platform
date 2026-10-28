// ABOUTME: Library half of the dev fixture: the local stand-in for Garmin's connect pages and its pushes
// ABOUTME: The binary serves it next to the seeded Strava routes; the server's tests drive the same code
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dev/test fixture library.
//!
//! [`garmin`] stands in for the Garmin Connect Developer Program in dev: its
//! `OAuth2` PKCE authorization and token endpoints, the `user/id` read and
//! deregistration, and the pushes Garmin sends to Dravr's webhook. The
//! binary serves it beside the seeded Strava routes, and the server's
//! integration tests run the same router and emitter end to end, so the
//! local flow and the tested flow cannot drift apart.

pub mod garmin;
