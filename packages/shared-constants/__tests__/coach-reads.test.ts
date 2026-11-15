// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The member's record of their human coach's reads: each kind of data a read reached names its own words
// ABOUTME: Every key the record asks for resolves in all five catalogues, so no client prints a raw key (carnet#1445)

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { COACH_READ_LOG_ALL_KEYS, coachReadClassKey, coachReadSourceKey } from '../src/coach-reads';

const LOCALES = ['en', 'fr', 'es', 'de', 'pt'] as const;

/** The value at a dotted key of a nested catalogue, or `undefined`. */
function lookup(catalogue: unknown, key: string): unknown {
  return key
    .split('.')
    .reduce<unknown>(
      (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
      catalogue,
    );
}

describe('coachReadClassKey', () => {
  it('names distinct words for training and wellness', () => {
    expect(coachReadClassKey('training')).toBe('humanCoach.readLog.training');
    expect(coachReadClassKey('wellness')).toBe('humanCoach.readLog.wellness');
  });
});

describe('coachReadSourceKey', () => {
  it('names words for each read made outside a tool call', () => {
    // The labels the server records in place of a tool (CoachReadSurface).
    for (const label of [
      'plan_view',
      'plan_command',
      'roster',
      'coach_thread',
      'group_room',
      'group_stats',
      'group_report',
      'group_health',
      'weekly_digest',
    ]) {
      expect(coachReadSourceKey(label)).toMatch(/^humanCoach\.readLog\.source/);
    }
  });

  it('leaves a tool name to be shown as it is', () => {
    expect(coachReadSourceKey('get_activities')).toBeNull();
    expect(coachReadSourceKey('toString')).toBeNull();
  });
});

describe('the catalogue', () => {
  it.each(LOCALES)('carries every key the record asks for in %s', (locale) => {
    // The catalogue is the data under test: the keys are authored in
    // dravr-contremaitre and copied here by the bump lane, so a key this
    // module names that the copy lacks would print raw on both clients.
    const path = resolve(__dirname, '../../i18n/src/locales', locale, 'translation.json');
    const catalogue: unknown = JSON.parse(readFileSync(path, 'utf8'));
    const missing = COACH_READ_LOG_ALL_KEYS.filter((key) => typeof lookup(catalogue, key) !== 'string');
    expect(missing).toEqual([]);
  });
});
