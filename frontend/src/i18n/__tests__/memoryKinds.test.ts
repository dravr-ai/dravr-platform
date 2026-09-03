// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the memory fact kind vocabulary — every server kind has a label key, and every key is in the corpus
// ABOUTME: A kind without a label rendered as a raw enum ("North_star") beside translated chrome; this keeps the table closed

import { describe, expect, it } from 'vitest';
import { MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import { MEMORY_FACT_KINDS } from '@pierre/shared-types';
import en from '../../../../packages/i18n/src/locales/en/translation.json';

function leaf(bundle: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>((node, part) => {
    return node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined;
  }, bundle);
}

describe('memory fact kinds', () => {
  it('is the vocabulary the label table is keyed by, with no kind unlabelled', () => {
    // The vocabulary and the label table used to be declared side by side in
    // one package, and this test compared them to a third hand-written copy —
    // so a kind added to the server could be missed by all three at once. The
    // table is now typed by the vocabulary, and this pins that every entry is
    // present rather than merely assignable.
    expect(Object.keys(MEMORY_KIND_LABEL_KEY).sort()).toEqual([...MEMORY_FACT_KINDS].sort());
  });

  it('names a corpus key for every kind, and the corpus carries it', () => {
    for (const kind of MEMORY_FACT_KINDS) {
      const key = MEMORY_KIND_LABEL_KEY[kind];
      expect(key, kind).toMatch(/^shell\.memoryKind/);
      expect(typeof leaf(en as Record<string, unknown>, key), key).toBe('string');
    }
  });
});
