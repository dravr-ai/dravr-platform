// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's binding of the "Who read your data" hooks — the feed, the Connected-apps list, and the one revoke both screens share
// ABOUTME: The feed's Revoke and the Connected apps screen read and reread the same keys (carnet#1543)

import { createAccessFeedHook, createConnectedAppsHook, createRevokeConnectedAppsHook } from '@pierre/ui-logic';
import { accessFeedApi, oauthApi } from '../services/api';

/** The athlete's feed of who read their data. The API is reached when the query runs, so importing it touches no client. */
export const useAccessFeed = createAccessFeedHook({
  getAccessFeed: () => accessFeedApi.getAccessFeed(),
});

/** The outside apps the athlete approved on the consent screen. */
export const useConnectedApps = createConnectedAppsHook({
  listConnectedApps: () => oauthApi.listConnectedApps(),
});

/** Revoke a Connected app's grants, then reread the Connected-apps list and the feed. */
export const useRevokeConnectedApps = createRevokeConnectedAppsHook({
  revokeConnectedApp: (id) => oauthApi.revokeConnectedApp(id),
});
