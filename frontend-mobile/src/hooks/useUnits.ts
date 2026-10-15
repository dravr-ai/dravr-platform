// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the athlete's units hook — the shared React Query hook over this client's athlete API
// ABOUTME: Metric unless the athlete chose imperial in Settings; the phone's locale plays no part (carnet#1044)

import { createUnitPreferencesHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

/**
 * The athlete's units. The API is reached when a read or write runs, so
 * importing it touches no client.
 */
export const useUnitPreferences = createUnitPreferencesHook({
  getUnitPreferences: () => athleteApi.getUnitPreferences(),
  updateUnitPreferences: (units) => athleteApi.updateUnitPreferences(units),
});
