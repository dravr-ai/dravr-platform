// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the sentence under the Settings units choice — what Automatic follows, and nothing while a choice decides
// ABOUTME: Red if the hint names a provider that did not decide, or speaks while the athlete's own choice wins (carnet#835)

import { describe, expect, it } from 'vitest';
import type { UnitPreferences } from '@pierre/shared-types';
import { UNIT_PREFERENCE_OPTIONS, unitsAutomaticHint } from '../src/unitsHook';

const BASE: UnitPreferences = {
  preference: 'automatic',
  units: 'imperial',
  source: 'provider',
  provider: 'strava',
  provider_units: 'imperial',
  device_locale: 'en-US',
};

describe('unitsAutomaticHint', () => {
  it('names the provider whose setting decided Automatic', () => {
    expect(unitsAutomaticHint(BASE)).toEqual({
      key: 'settings.unitsFromProvider',
      systemKey: 'settings.unitsSystemImperial',
      provider: 'Strava',
    });
  });

  it('names the device region when no provider setting is known', () => {
    expect(unitsAutomaticHint({ ...BASE, source: 'locale', provider: null, provider_units: null, units: 'metric' })).toEqual({
      key: 'settings.unitsFromLocale',
      systemKey: 'settings.unitsSystemMetric',
    });
  });

  it('says nothing while the athlete\'s own choice decides', () => {
    expect(unitsAutomaticHint({ ...BASE, preference: 'metric', units: 'metric', source: 'override' })).toBeNull();
  });

  it('offers Automatic first, then Metric and Imperial', () => {
    expect(UNIT_PREFERENCE_OPTIONS.map((option) => option.value)).toEqual(['automatic', 'metric', 'imperial']);
  });
});
