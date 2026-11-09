// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web binding of the preferred-calendar hook — the shared React Query hook over this client's athlete API
// ABOUTME: The calendar Dravr's plan and workout pushes go to, chosen in Connections settings (carnet#1393)

import { createPreferredCalendarHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

/**
 * The athlete's preferred calendar. The API is reached when a read or write
 * runs, so importing it touches no client.
 */
export const usePreferredCalendar = createPreferredCalendarHook({
  getPreferredCalendar: () => athleteApi.getPreferredCalendar(),
  setPreferredCalendar: (provider) => athleteApi.setPreferredCalendar(provider),
  clearPreferredCalendar: () => athleteApi.clearPreferredCalendar(),
});
