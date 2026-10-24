// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of Home's reads — the shared ui-logic hooks over this client's athlete API, plus the provider status
// ABOUTME: A stale activity answer is followed up on the HOME_STALE_REFETCH_DELAYS_MS schedule, which ends, and only while the app is in use

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { createActivityUploadHook, createHomeReadHooks, createHomePreferencesHook } from '@pierre/ui-logic';
import { athleteApi, oauthApi } from '../services/api';

/**
 * Home's reads over the mobile client's athlete API — the recent activities
 * and their stale-answer schedule, one activity's route, the plan, the
 * training status and the weekly volume. The definitions live in
 * `@pierre/ui-logic`, one for web and mobile; the API is reached when a read
 * runs, so importing this touches no client.
 */
export const { useRecentActivities, useActivityRoute, useTrainingPlan, useTrainingStatus, useTrainingVolume } =
  createHomeReadHooks(
    {
      getRecentActivities: (...args) => athleteApi.getRecentActivities(...args),
      getActivityRoute: (provider, activityId, options) => athleteApi.getActivityRoute(provider, activityId, options),
      getTrainingPlan: (locale) => athleteApi.getTrainingPlan(locale),
      getTrainingStatus: () => athleteApi.getTrainingStatus(),
      getTrainingVolume: () => athleteApi.getTrainingVolume(),
    },
    { useLanguage: () => useTranslation().language },
  );

/**
 * Whether any fitness provider is connected, whether one of them still
 * syncs, and which connected ones have to be reconnected, from the provider
 * status the Connections pane reads — the activity list carries none of it.
 *
 * `connected` is `null` until the status answers; a status read that fails
 * reads as connected, because the empty sentence ("they show up once your
 * provider syncs") is true either way while telling a connected athlete to
 * connect is not.
 *
 * `needsReconnect` names the connected providers flagged `needs_reauth`, in
 * the order the server lists them. The server refreshes nothing for such a
 * connection, so its activities stay as old as its last sync. The flag only
 * means something on a connected provider, so a disconnected one is never
 * named; and two rows can carry one name — the `sciotte` mirror and the
 * `strava` OAuth row are both "Strava" — so a name is listed once. Nor is one
 * whose sign-in is paused (`connect_paused`): it cannot be reconnected, and
 * its Connections row says why.
 */
export function useProviderConnected() {
  const query = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => oauthApi.getProvidersStatus(),
  });

  let connected: boolean | null = null;
  if (query.data !== undefined) {
    connected = query.data.providers.some((provider) => provider.connected);
  } else if (query.isError) {
    connected = true;
  }

  const needsReconnect = [
    ...new Set(
      (query.data?.providers ?? [])
        .filter((provider) => provider.connected && provider.needs_reauth && !provider.connect_paused)
        .map((provider) => provider.display_name),
    ),
  ];

  // Whether a connected provider is still syncing — one not flagged. Same
  // reading as `connected`: `null` until the status answers, and a failed
  // read counts as syncing, since nothing then says otherwise.
  let syncing: boolean | null = null;
  if (query.data !== undefined) {
    syncing = query.data.providers.some((provider) => provider.connected && !provider.needs_reauth);
  } else if (query.isError) {
    syncing = true;
  }

  return {
    connected,
    syncing,
    needsReconnect,
    // The rows themselves, for a reader that words them its own way — the
    // thread header. `null` until the status answers.
    providers: query.data?.providers ?? null,
    refetch: query.refetch,
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
