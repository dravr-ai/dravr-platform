// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web binding of Home's reads — the shared ui-logic hooks over this client's athlete API, plus the provider status
// ABOUTME: A stale activity answer is followed up on a schedule that ends, and only while somebody is using the tab; never polled

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { createActivityUploadHook, createHomeReadHooks, createHomePreferencesHook } from '@pierre/ui-logic';
import { athleteApi, providersApi } from '../services/api';

/**
 * Home's reads over the web client's athlete API — the recent activities and
 * their stale-answer schedule, one activity's route, the plan, the training
 * status and the weekly volume. The definitions live in `@pierre/ui-logic`,
 * one for web and mobile; the API is reached when a read runs, so importing
 * this touches no client.
 */
export const {
  recentActivitiesQuery,
  useRecentActivities,
  useActivityRoute,
  useTrainingPlan,
  useTrainingStatus,
  useTrainingVolume,
} = createHomeReadHooks(
  {
    getRecentActivities: (...args) => athleteApi.getRecentActivities(...args),
    getActivityRoute: (provider, activityId, options) => athleteApi.getActivityRoute(provider, activityId, options),
    getTrainingPlan: (locale) => athleteApi.getTrainingPlan(locale),
    getTrainingStatus: () => athleteApi.getTrainingStatus(),
    getTrainingVolume: () => athleteApi.getTrainingVolume(),
  },
  { useLanguage: () => useTranslation().language },
);

/** What the Home page reads from the provider-status query. */
export interface ProviderConnectionState {
  loaded: boolean;
  connected: boolean;
  /**
   * The display name of every connected provider whose session is not usable
   * until the athlete reconnects it, in the order the server lists them and
   * each name once: a provider served by a mirror backend is listed as two
   * cards that share one name.
   */
  needsReconnect: string[];
}

/**
 * Whether the athlete has any provider connected, and which of the connected
 * ones have to be reconnected, read from the same provider-status query the
 * chat pane and the connect banner share.
 *
 * `loaded` is false until that query answers, so a connected athlete is never
 * shown a connect prompt while the answer is still in flight.
 *
 * The server refreshes nothing for a connection flagged `needs_reauth`, so
 * its activities stay as old as its last sync. The flag only means something
 * on a connected provider, which is why a disconnected one is never named.
 */
export function useProviderConnection(): ProviderConnectionState {
  const { data, isSuccess } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  const providers = data?.providers ?? [];
  return {
    loaded: isSuccess,
    connected: providers.some((provider) => provider.connected),
    needsReconnect: [
      ...new Set(
        providers.filter((provider) => provider.connected && provider.needs_reauth).map((provider) => provider.display_name),
      ),
    ],
  };
}

/**
 * What the athlete chose about Home — whether it still offers to build a
 * plan. Stored on the server, so the phone and the web agree. The API is
 * reached when a read or write runs, like every other hook in this module,
 * so importing it touches no client.
 */
export const useHomePreferences = createHomePreferencesHook({
  getHomePreferences: () => athleteApi.getHomePreferences(),
  updateHomePreferences: (prefs) => athleteApi.updateHomePreferences(prefs),
});

/**
 * Upload the `.fit` file of a completed workout; the new activity joins the
 * recent list, the calendar and the volume, which this refreshes. The API is
 * reached when an upload runs, so importing it touches no client.
 */
export const useActivityUpload = createActivityUploadHook({
  uploadActivityFile: (bytes) => athleteApi.uploadActivityFile(bytes),
});
