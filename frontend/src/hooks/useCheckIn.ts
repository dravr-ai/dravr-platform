// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web binding of the check-in hook — the shared React Query hook over this client's athlete API
// ABOUTME: Read and written by the onboarding step and the Settings control alike (carnet#831)

import { createCheckInHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

/**
 * How often the coach checks in on its own. The API is reached when a read
 * or write runs, so importing it touches no client.
 */
export const useCheckIn = createCheckInHook({
  getCheckIn: () => athleteApi.getCheckIn(),
  updateCheckIn: (cadence) => athleteApi.updateCheckIn(cadence),
});
