// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Build-time feature toggles for surfaces not yet shipping (web)
// ABOUTME: Flip BILLING_ENABLED to true to re-expose the billing UI entry points

/**
 * Billing is out of scope for the first release. While false, the billing nav
 * entry points (admin Billing tab, user Usage tab) are hidden so the billing
 * UI is unreachable. Set to true to re-enable the billing surface.
 */
export const BILLING_ENABLED = false;
