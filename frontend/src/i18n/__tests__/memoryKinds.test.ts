// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the memory fact kind vocabulary — every server kind has a label key, and every key is in the corpus
// ABOUTME: A kind without a label rendered as a raw enum ("North_star") beside translated chrome; this keeps the table closed

import { describe, expect, it } from 'vitest';
import { MEMORY_FACT_KINDS, MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import en from '../../../../packages/i18n/src/locales/en/translation.json';

/** The nine `FactKind` serde strings `crates/pierre-memory/src/facts.rs` declares. */
const SERVER_KINDS = [
  'preference',
  'physiology',
  'injury',
  'goal',
  'schedule',
  'equipment',
  'north_star',
  'medical',
  'other',
];

function leaf(bundle: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>((node, part) => {
    return node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined;
  }, bundle);
}

describe('memory fact kinds', () => {
  it('lists exactly the kinds the server can send', () => {
    expect([...MEMORY_FACT_KINDS]).toEqual(SERVER_KINDS);
  });

  it('names a corpus key for every kind, and the corpus carries it', () => {
    for (const kind of MEMORY_FACT_KINDS) {
      const key = MEMORY_KIND_LABEL_KEY[kind];
      expect(key, kind).toMatch(/^shell\.memoryKind/);
      expect(typeof leaf(en as Record<string, unknown>, key), key).toBe('string');
    }
  });
});
