// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One activity's live map — the chat's own route card with its layers and full screen — or the sentence for why there is none
// ABOUTME: Home's latest card and the activity screen draw the same thing: loading, a map, no GPS, too short, or could not be loaded with a retry

import React from 'react';
import { Text, View } from 'react-native';
import type { HomeActivity } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState } from '../../components/ui';
import { useActivityRoute } from '../../hooks/useHome';
import { LazyRouteView } from '../chat/SceneView';

/** A sentence where the map would be — why there is none, or that it is on its way. */
function MapNote({ children, testID }: { children: string; testID: string }) {
  return (
    <Text className="px-4 py-3 text-sm text-text-secondary" testID={testID}>
      {children}
    </Text>
  );
}

/**
 * The activity's map. An activity that says `has_gps: false` had its route
 * read once and the recording held no GPS: it says so and costs no request.
 * Every other asks the route endpoint — its route may never have been read —
 * and the answer is what says whether there is a track: a route to draw, or
 * the reason there is none. The route arrives privacy-trimmed from the
 * server and is drawn by the chat's own route card, loaded on demand behind
 * the boundary that keeps a runtime without MapLibre (Expo Go) showing a
 * sentence instead of losing the screen.
 *
 * `testIDPrefix` names each state for the screen that draws it:
 * `<prefix>-map`, `<prefix>-no-track`, `<prefix>-map-loading`, …
 * `burst` marks Home's map, one of its screen's burst of route reads.
 */
export function ActivityMap({
  activity,
  testIDPrefix,
  burst = false,
}: {
  activity: Pick<HomeActivity, 'provider' | 'id' | 'has_gps'>;
  testIDPrefix: string;
  /** The map is Home's, read in the screen's burst of route reads; the activity screen's is not. */
  burst?: boolean;
}) {
  const { t } = useTranslation();
  const route = useActivityRoute(activity.provider, activity.id, activity.has_gps, { burst });

  if (!activity.has_gps || route.reason === 'no_gps') {
    return <MapNote testID={`${testIDPrefix}-no-track`}>{t('chat.routeNoTrack')}</MapNote>;
  }
  if (route.reason === 'too_short') {
    return <MapNote testID={`${testIDPrefix}-too-short`}>{t('home.activities.routeTooShort')}</MapNote>;
  }
  if (route.route !== null) {
    return (
      <View className="px-4" testID={`${testIDPrefix}-map`}>
        <LazyRouteView
          route={route.route}
          fallback={<MapNote testID={`${testIDPrefix}-map-loading`}>{t('home.activities.mapLoading')}</MapNote>}
          unavailable={
            <MapNote testID={`${testIDPrefix}-map-unavailable`}>{t('home.activities.routeFailed')}</MapNote>
          }
        />
      </View>
    );
  }
  // A read that failed, or one the server could not make just now
  // (`unavailable`), is the same sentence and the same retry: neither says
  // anything about whether the activity recorded a route. While a read is in
  // flight — the retry's included — the map says it is loading, so a retry
  // is seen to do something.
  if ((route.isError || route.reason === 'unavailable') && !route.isFetching) {
    return (
      <EmptyState
        action={{ label: t('common.retry'), onPress: route.retry, testID: `${testIDPrefix}-map-retry` }}
        testID={`${testIDPrefix}-map-failed`}
      >
        {t('home.activities.routeFailed')}
      </EmptyState>
    );
  }
  // No answer yet: the read is in flight, or paused while the phone is offline.
  return <MapNote testID={`${testIDPrefix}-map-loading`}>{t('home.activities.mapLoading')}</MapNote>;
}
