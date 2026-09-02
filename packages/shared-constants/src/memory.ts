// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The memory fact kind vocabulary both memory screens read — the filter and the group badge share one word
// ABOUTME: Mirrors the server's FactKind serde strings; labels are corpus keys, resolved with t() at render

/**
 * Every `kind` a user_facts row can carry, in the order the server's
 * `FactKind` enum declares them (`crates/pierre-memory/src/facts.rs`). The
 * strings are the wire values — what the server serializes and what the
 * filter sends back — so they stay English whatever the athlete reads.
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

export type MemoryFactKind = (typeof MEMORY_FACT_KINDS)[number];

/**
 * The corpus key naming each kind. Module scope cannot hold a hook, so the
 * table carries the key and each client resolves it with its own `t` — the
 * filter dropdown and the group badge then show the same word, in the
 * athlete's language, instead of one translated word beside a raw enum.
 */
export const MEMORY_KIND_LABEL_KEY: Record<MemoryFactKind, string> = {
  preference: 'shell.memoryKindPreference',
  physiology: 'shell.memoryKindPhysiology',
  injury: 'shell.memoryKindInjury',
  goal: 'shell.memoryKindGoal',
  schedule: 'shell.memoryKindSchedule',
  equipment: 'shell.memoryKindEquipment',
  north_star: 'shell.memoryKindNorthStar',
  medical: 'shell.memoryKindMedical',
  other: 'shell.memoryKindOther',
};
