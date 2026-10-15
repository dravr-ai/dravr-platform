// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings units choice — Metric then Imperial, and no Automatic that could infer miles (carnet#1044)
// ABOUTME: Red if an option comes back that lets anything but the athlete's own choice put them on imperial

import { describe, expect, it } from 'vitest';
import { UNIT_SYSTEM_OPTIONS } from '../src/unitsHook';

describe('UNIT_SYSTEM_OPTIONS', () => {
  it('offers Metric first, then Imperial, and nothing else', () => {
    expect(UNIT_SYSTEM_OPTIONS.map((option) => option.value)).toEqual(['metric', 'imperial']);
    expect(UNIT_SYSTEM_OPTIONS.map((option) => option.labelKey)).toEqual(['settings.unitsMetric', 'settings.unitsImperial']);
  });
});
