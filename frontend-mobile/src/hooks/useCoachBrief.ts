// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the coach's morning brief — the shared ui-logic hooks over this client's coach roster API (carnet#1791)
// ABOUTME: The brief is read and changed with a cache patch before the request lands; opening an athlete pings once, never blocking

import { createCoachBriefHooks } from '@pierre/ui-logic';
import { coachRosterApi } from '../services/api';

/**
 * The brief's hooks over the mobile client's coach roster API. The
 * definitions live in `@pierre/ui-logic`, one for web and mobile; the API is
 * reached when a hook runs, so importing this touches no client.
 */
export const { useCoachBrief, useUpdateCoachBrief, useBriefOpenedPing } = createCoachBriefHooks({
  getBrief: () => coachRosterApi.getBrief(),
  updateBrief: (update) => coachRosterApi.updateBrief(update),
  markBriefOpened: (athleteId) => coachRosterApi.markBriefOpened(athleteId),
});
