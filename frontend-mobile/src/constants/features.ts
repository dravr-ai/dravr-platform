// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Build-time feature toggles for surfaces not yet shipping (mobile)
// ABOUTME: Flip BILLING_ENABLED to true to re-expose the billing UI entry points

/**
 * Billing is out of scope for the first release. While false, the billing route
 * is unreachable (it redirects away) and any billing entry points stay hidden.
 * Set to true to re-enable the billing surface across the app.
 */
export const BILLING_ENABLED = false;
