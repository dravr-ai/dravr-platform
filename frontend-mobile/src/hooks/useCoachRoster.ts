// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of Coach Home's reads and writes — the shared ui-logic hooks over this client's coach roster API
// ABOUTME: The roster is read page by page and merged; "Traité" and its undo patch the cache before the request lands

import { createCoachRosterHooks } from '@pierre/ui-logic';
import { coachRosterApi } from '../services/api';

/**
 * Coach Home's hooks over the mobile client's coach roster API. The
 * definitions live in `@pierre/ui-logic`, one for web and mobile; the API is
 * reached when a hook runs, so importing this touches no client.
 */
export const { useCoachRoster, useDismissAttention, useRestoreAttention } = createCoachRosterHooks({
  getRoster: (query) => coachRosterApi.getRoster(query),
  dismissAttention: (request) => coachRosterApi.dismissAttention(request),
  restoreAttention: (request) => coachRosterApi.restoreAttention(request),
});
