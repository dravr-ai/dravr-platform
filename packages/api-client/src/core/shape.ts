// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one check every parsed answer passes through: the body the shared-types parser accepted, or an error naming the endpoint
// ABOUTME: Shared by the domains whose reads are parsed, so a malformed answer fails the same way on every screen

/**
 * The parsed body, or an error naming the endpoint whose answer broke the
 * contract — the query then shows its error state instead of a card built
 * from a shape nobody agreed to.
 */
export function requireShape<T>(parsed: T | null, endpoint: string): T {
  if (parsed === null) {
    throw new Error(`${endpoint} answered with a body that does not match its contract`);
  }
  return parsed;
}
