// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The memory fact kind vocabulary — the wire values the server's FactKind serializes
// ABOUTME: Lives in shared-types because api-client types a row with it and shared-constants labels it

/**
 * Every `kind` a `user_facts` row can carry, in the order the server's
 * `FactKind` enum declares them (`crates/pierre-memory/src/facts.rs`).
 *
 * These are wire values — what the server serializes and what a filter sends
 * back — so they stay English whatever the athlete reads. The words shown to
 * an athlete are in `@pierre/shared-constants`, keyed by these.
 *
 * It lives here, in the package with no dependencies, because both consumers
 * sit downstream of it: `@pierre/api-client` types `MemoryFactRow.kind` and
 * `@pierre/shared-constants` maps each to a corpus key. Spelled in each of
 * them instead, the nine strings appeared three times — twice as a list and
 * once as an inline union — and the test meant to catch that drift compared
 * two of the copies to a fourth, hand-written in the test itself.
 */
export const MEMORY_FACT_KINDS = [
  'preference',
  'physiology',
  'injury',
  'goal',
  'schedule',
  'equipment',
  'north_star',
  'medical',
  'other',
] as const;

/** One of {@link MEMORY_FACT_KINDS}. */
export type MemoryFactKind = (typeof MEMORY_FACT_KINDS)[number];
