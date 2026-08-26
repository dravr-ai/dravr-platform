// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Build-time feature toggles for surfaces not yet shipping (mobile)
// ABOUTME: Set EXPO_PUBLIC_BILLING_ENABLED=true to re-expose the billing UI entry points

/**
 * Billing is out of scope for the first release. While false, the billing route
 * is unreachable (it redirects away) and any billing entry points stay hidden.
 * Defaults to false; set the build-time env var `EXPO_PUBLIC_BILLING_ENABLED=true`
 * to re-expose the billing surface.
 *
 * This is the `EXPO_PUBLIC_` counterpart of the web's `VITE_BILLING_ENABLED`.
 * It used to be a hardcoded `false`, which meant the two platforms disagreed
 * about what kind of thing the flag was: web could arm billing for a build or a
 * test run, mobile could only arm it by editing source. Nothing about the
 * platforms required that — Expo inlines `EXPO_PUBLIC_*` at build time exactly
 * as Vite inlines `VITE_*`.
 */
export const BILLING_ENABLED = process.env.EXPO_PUBLIC_BILLING_ENABLED === 'true';
