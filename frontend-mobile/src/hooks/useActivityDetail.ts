// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the shared activity-detail read to this client's athlete API
// ABOUTME: One workout's view — its Home row, the figures the cache holds, its splits and laps

import { createActivityDetailHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

export const useActivityDetail = createActivityDetailHook(athleteApi);
