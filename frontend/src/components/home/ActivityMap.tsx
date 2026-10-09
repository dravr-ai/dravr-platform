// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One activity's live map — the chat's own map component with its layers and full screen — or the words for why there is none
// ABOUTME: Home's latest card and the activity view draw the same thing: loading, a map, no GPS, too short, or could not be loaded with a retry

import type { ReactNode } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { HomeActivity } from '@pierre/shared-types';
import { sportIsOnFoot } from '@pierre/shared-constants';
import { useDistanceUnit } from '@pierre/ui-logic';
import { EmptyState } from '../ui/EmptyState';
import RouteView from '../chat/RouteView';
import { useActivityRoute } from '../../hooks/useHome';
import { FEATURE_KEYS, useFeatureFlags } from '../../hooks/useFeatureFlags';

/** The frame the map fills, holding a line of text while there is no map in it. */
function MapNote({ children, compact }: { children: ReactNode; compact: boolean }) {
  return (
    <div
      className={`my-4 flex w-full items-center justify-center rounded-[10px] border ghost-border bg-surface-container-lowest px-4 text-center text-sm text-on-surface-variant ${
        compact ? 'h-48' : 'h-64 sm:h-80'
      }`}
    >
      {children}
    </div>
  );
}

/**
 * The activity's map. `has_gps: false` says the route was read once and the
 * recording held no GPS, so it costs no request. Every other activity asks
 * the route endpoint once — its route may never have been read, and the
 * answer is what says whether there is a track — and draws what comes back
 * with the chat's own map component. `burst` marks Home's map, one of its
 * page's burst of route reads.
 *
 * A run, trail run, walk or hike is drawn with its start, finish and distance
 * marks — the beta the `route_km_markers` flag arms per athlete — counted in
 * kilometres or miles, the athlete's own units (carnet#835).
 */
export function ActivityMap({
  activity,
  burst = false,
  compact = false,
}: {
  activity: Pick<HomeActivity, 'provider' | 'id' | 'has_gps' | 'sport_type'>;
  /** The map is Home's, read in the page's burst of route reads; the activity view's is not. */
  burst?: boolean;
  /** A shorter frame, for Home's Today panel beside the conversation. */
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const route = useActivityRoute(activity.provider, activity.id, activity.has_gps, { burst });
  const { flags } = useFeatureFlags();
  const unit = useDistanceUnit();
  const markerUnit = flags[FEATURE_KEYS.routeKmMarkers] && sportIsOnFoot(activity.sport_type) ? unit : null;

  if (!activity.has_gps) {
    return <p className="py-3 text-sm text-on-surface-variant">{t('chat.routeNoTrack')}</p>;
  }
  // A read that failed, or one the server could not make just now
  // (`unavailable`), is the same sentence and the same retry: neither says
  // anything about whether the activity recorded a route. While a read is
  // in flight — the retry's included — the map says it is loading, so a
  // retry is seen to do something.
  if (route.data === undefined || route.data.reason === 'unavailable') {
    const failed = route.isError || route.data?.reason === 'unavailable';
    return failed && !route.isFetching ? (
      <EmptyState
        data-testid="home-route-failed"
        action={{ label: t('common.retry'), onClick: route.retry }}
      >
        {t('home.activities.routeFailed')}
      </EmptyState>
    ) : (
      <MapNote compact={compact}>
        <span role="status">{t('home.activities.mapLoading')}</span>
      </MapNote>
    );
  }
  if (route.data.route !== null) {
    return <RouteView view={route.data.route} compact={compact} markerUnit={markerUnit} />;
  }
  return (
    <p className="py-3 text-sm text-on-surface-variant">
      {route.data.reason === 'too_short' ? t('home.activities.routeTooShort') : t('chat.routeNoTrack')}
    </p>
  );
}
