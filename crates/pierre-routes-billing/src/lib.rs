// ABOUTME: Provider-agnostic billing route group — checkout, portal, webhook, subscription, invoices
// ABOUTME: Generic over BillingCtx + MiddlewareCtx so the crate stays decoupled from pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Billing Routes
//!
//! Hosts the `/api/billing/{checkout,portal,subscription,invoices}` REST
//! endpoints and the `/webhooks/{provider}` receiver, dispatching to the
//! active [`pierre_core::billing::BillingProvider`] for vendor-specific
//! work (hosted session creation, webhook signature verification, payload
//! normalization). Subscription row mutations and tier flips happen here
//! via the repository registry; idempotency is enforced by the
//! `billing_events` table keyed on `(provider, event_id)`.
//!
//! The route group is generic over [`pierre_runtime_context::BillingCtx`]
//! (for provider + repos access) and [`pierre_runtime_context::MiddlewareCtx`]
//! (for the `AuthenticatedUser` extractor); the composition root in
//! `pierre-server` implements both traits on its `ServerContext`.

#![warn(missing_docs)]

/// Billing endpoints + webhook dispatcher.
pub mod billing;

pub use billing::{billing_routes, dispatch_billing_event, plan_catalog, PlanView, PlansResponse};
