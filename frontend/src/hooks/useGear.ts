// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the shared gear ledger hooks to this client's gear API
// ABOUTME: The shoes the athlete declares in Settings, and the pair an activity's view says it was run in

import { createGearHooks } from '@pierre/ui-logic';
import { gearApi } from '../services/api';

/**
 * The API is reached when a hook reads or writes, never when this module is
 * imported, so a view that imports the activity picker touches no client
 * until the picker is drawn.
 */
export const { useGearLedger, useGearChanges, useActivityGear } = createGearHooks({
  listGear: () => gearApi.listGear(),
  createGear: (request) => gearApi.createGear(request),
  updateGear: (gearId, changes) => gearApi.updateGear(gearId, changes),
  retireGear: (gearId) => gearApi.retireGear(gearId),
  restoreGear: (gearId) => gearApi.restoreGear(gearId),
  deleteGear: (gearId) => gearApi.deleteGear(gearId),
  getActivityGear: (provider, activityId) => gearApi.getActivityGear(provider, activityId),
  setActivityGear: (provider, activityId, gearId) => gearApi.setActivityGear(provider, activityId, gearId),
});
