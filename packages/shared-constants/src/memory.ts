// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The memory fact kind vocabulary both memory screens read — the filter and the group badge share one word
// ABOUTME: Mirrors the server's FactKind serde strings; labels are corpus keys, resolved with t() at render

import { countKey } from '@pierre/chat-utils';
import type { MemoryFactKind } from '@pierre/shared-types';
import { MEMORY_FACT_COUNT_KEYS } from './count-keys';

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

/**
 * How many facts a memory screen counts, « 0 info », « 1 info », « 4 infos »:
 * both screens print it through this, so [`countKey`] picks the form the same
 * way on each.
 */
export function memoryFactCountLabel(
  t: (key: string, options: { count: number }) => string,
  count: number,
  language: string | undefined,
): string {
  return t(countKey(count, language, MEMORY_FACT_COUNT_KEYS), { count });
}
