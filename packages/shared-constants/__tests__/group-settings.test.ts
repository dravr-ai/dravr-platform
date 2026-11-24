// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the weekly-digest modes both clients offer, their order and their catalogue keys, and who manages a group
// ABOUTME: Red if a mode lacks a label or hint in any locale, or a plain member is let into a group's management

import { describe, expect, it } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { DIGEST_MODES, DIGEST_MODE_ORDER, canManageGroup } from '../src/groups';

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node !== null && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('DIGEST_MODES', () => {
  it('offers every mode once, off first', () => {
    expect(DIGEST_MODE_ORDER).toEqual(['off', 'chat', 'managers']);
    expect([...DIGEST_MODE_ORDER].sort()).toEqual(Object.keys(DIGEST_MODES).sort());
  });

  it('names a label and a hint every locale carries', () => {
    for (const mode of DIGEST_MODE_ORDER) {
      for (const catalogue of [en, fr, es, de, pt]) {
        expect(typeof lookup(catalogue, DIGEST_MODES[mode].labelKey)).toBe('string');
        expect(typeof lookup(catalogue, DIGEST_MODES[mode].hintKey)).toBe('string');
      }
    }
  });
});

describe('canManageGroup', () => {
  it('lets the owner and an admin manage, and nobody else', () => {
    expect(canManageGroup('owner')).toBe(true);
    expect(canManageGroup('admin')).toBe(true);
    expect(canManageGroup('member')).toBe(false);
    expect(canManageGroup(undefined)).toBe(false);
    expect(canManageGroup(null)).toBe(false);
  });
});
