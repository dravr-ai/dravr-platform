// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the shared activity-detail read and the uploaded-activity delete to this client's athlete API
// ABOUTME: One workout's view — its Home row, the figures the cache holds, its splits and laps — and its removal when uploaded

import { createActivityDetailHook, createDeleteUploadedActivityHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

export const useActivityDetail = createActivityDetailHook(athleteApi);

/**
 * Delete one of the athlete's uploaded activities; the API is reached when a
 * delete runs, so importing it touches no client.
 */
export const useDeleteUploadedActivity = createDeleteUploadedActivityHook({
  deleteUploadedActivity: (activityId) => athleteApi.deleteUploadedActivity(activityId),
});
