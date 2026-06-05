// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Build-time feature toggles for surfaces not yet shipping (web)
// ABOUTME: Set VITE_BILLING_ENABLED=true to re-expose the billing UI entry points

/**
 * Billing is out of scope for the first release. While false, the billing nav
 * entry points (admin Billing tab, user Usage tab) are hidden so the billing
 * UI is unreachable. Defaults to false; set the build-time env var
 * `VITE_BILLING_ENABLED=true` to re-expose the billing surface (the E2E suite
 * does this so the billing UI stays test-covered while it ships disabled).
 */
export const BILLING_ENABLED = import.meta.env.VITE_BILLING_ENABLED === 'true';
