// ABOUTME: Reserved module for message routing to Pierre chat pipeline
// ABOUTME: Routing logic is currently handled by webhooks.rs in pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Message routing from inbound webhooks to the Pierre chat pipeline is handled
// directly in pierre-server's webhook handler (routes/messaging/webhooks.rs).
// This module is reserved for future extraction of that logic into a reusable
// routing component within the messaging crate.
