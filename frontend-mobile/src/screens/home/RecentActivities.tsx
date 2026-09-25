// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home "Recent activities" section — the latest on a live map, the four before it with a route sketch
// ABOUTME: An indoor activity says it has no track, a stale cache says it is checking, and a tap opens a chat drafted about the activity

import React from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import type { HomeActivity } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useActivityRoute, type StaleRefetchPhase } from '../../hooks/useHome';
import { LazyRouteView } from '../chat/SceneView';
import { ActivitySketch } from './RouteSketch';
import { activityDraft, activityFigures, instantShortDate, sportLabel, syncedAtLabel } from './homeFormat';

type OpenDraft = (draft: string) => void;

/** A sentence where the map would be — why there is none, or that it is on its way. */
function MapNote({ children, testID }: { children: string; testID: string }) {
  return (
    <Text className="px-4 py-3 text-sm text-text-secondary" testID={testID}>
      {children}
    </Text>
  );
}

/**
 * The latest activity's map. Only an activity recorded with GPS asks for its
 * route; one without says so and costs no request. The route arrives
 * privacy-trimmed from the server and is drawn by the chat's own route card,
 * loaded on demand behind the boundary that keeps a runtime without MapLibre
 * (Expo Go) showing a sentence instead of losing the screen.
 */
function LatestMap({ activity }: { activity: HomeActivity }) {
  const { t } = useTranslation();
  const route = useActivityRoute(activity.provider, activity.id, activity.has_gps);

  if (!activity.has_gps || route.reason === 'no_gps') {
    return <MapNote testID="home-latest-no-track">{t('chat.routeNoTrack')}</MapNote>;
  }
  if (route.reason === 'too_short') {
    return <MapNote testID="home-latest-too-short">{t('home.activities.routeTooShort')}</MapNote>;
  }
  if (route.route !== null) {
    return (
      <View className="px-4" testID="home-latest-map">
        <LazyRouteView
          route={route.route}
          fallback={<MapNote testID="home-latest-map-loading">{t('home.activities.mapLoading')}</MapNote>}
          unavailable={<MapNote testID="home-latest-map-unavailable">{t('home.activities.routeFailed')}</MapNote>}
        />
      </View>
    );
  }
  if (route.isError) {
    return (
      <EmptyState
        action={{ label: t('common.retry'), onPress: () => void route.refetch(), testID: 'home-latest-map-retry' }}
        testID="home-latest-map-failed"
      >
        {t('home.activities.routeFailed')}
      </EmptyState>
    );
  }
  // No answer yet: the read is in flight, or paused while the phone is offline.
  return <MapNote testID="home-latest-map-loading">{t('home.activities.mapLoading')}</MapNote>;
}

/** "Sat 20 Sep · Ride · 92.0 km · 3h 41m 5s" — the row's second line, its figures in mono. */
function ActivityFacts({ activity }: { activity: HomeActivity }) {
  const { t, language } = useTranslation();
  return (
    <Text className="text-sm text-text-secondary" numberOfLines={1}>
      <Text className="font-mono tabular-nums">{instantShortDate(activity.start_date, language)}</Text>
      {' · '}
      {sportLabel(t, activity.sport_type)}
      {activityFigures(activity).map((figure, index) => (
        // Positional: the figures are distance, time and climb, in that order.
        <Text key={index}>
          {' · '}
          <Text className="font-mono tabular-nums">{figure}</Text>
        </Text>
      ))}
    </Text>
  );
}

/** One activity's words, pressable into a new chat drafted about it. */
function ActivityButton({
  activity,
  openDraft,
  leading,
  testID,
}: {
  activity: HomeActivity;
  openDraft: OpenDraft;
  leading?: React.ReactNode;
  testID: string;
}) {
  const { t, language } = useTranslation();
  return (
    <Pressable
      onPress={() => openDraft(activityDraft(t, activity, language))}
      accessibilityRole="button"
      className="flex-row items-center px-4 py-2 min-h-11"
      testID={testID}
    >
      {leading}
      <View className={`flex-1 min-w-0 ${leading ? 'ml-3' : ''}`}>
        <Text className="text-base text-text-primary" numberOfLines={1}>
          {activity.name}
        </Text>
        <ActivityFacts activity={activity} />
      </View>
    </Pressable>
  );
}

/**
 * Where the list stands against the provider: checking while a stale answer's
 * follow-up is pending, otherwise when it last synced — the web page's rule.
 */
function SyncLine({
  stale,
  staleRefetch,
  asOf,
}: {
  stale: boolean;
  staleRefetch: StaleRefetchPhase;
  asOf: string | null;
}) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  if (stale && staleRefetch !== 'done') {
    return (
      <View className="flex-row items-center px-4 pb-2" testID="home-activities-refreshing">
        <ActivityIndicator size="small" color={colors.text.secondary} />
        <Text className="ml-2 text-sm text-text-secondary">{t('home.activities.refreshing')}</Text>
      </View>
    );
  }
  if (asOf === null) {
    return null;
  }
  return (
    <Text className="px-4 pb-2 text-sm text-text-secondary" testID="home-activities-synced-at">
      {t('home.activities.syncedAt', { time: syncedAtLabel(asOf, language) })}
    </Text>
  );
}

interface RecentActivitiesProps {
  activities: HomeActivity[];
  /** False until the first answer; stays false while a read is pending, paused offline, or failed. */
  hasData: boolean;
  isError: boolean;
  stale: boolean;
  staleRefetch: StaleRefetchPhase;
  asOf: string | null;
  onRetry: () => void;
  /**
   * Whether any fitness provider is connected, from the provider status —
   * the activity list carries no such flag. `null` until the status answers.
   */
  providerConnected: boolean | null;
  onConnect: () => void;
  openDraft: OpenDraft;
}

/**
 * The newest five activities: the latest on its map, the rest as rows with a
 * sketch of their route. An empty list says why in one sentence — no
 * provider to read from, with the way to connect one, or nothing synced yet.
 */
export function RecentActivities({
  activities,
  hasData,
  isError,
  stale,
  staleRefetch,
  asOf,
  onRetry,
  providerConnected,
  onConnect,
  openDraft,
}: RecentActivitiesProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();

  let body: React.ReactNode;
  if (!hasData) {
    body = isError ? (
      <EmptyState
        action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-activities-retry' }}
        testID="home-activities-error"
      >
        {t('home.activities.loadFailed')}
      </EmptyState>
    ) : (
      <View className="px-4 py-3 items-start" testID="home-activities-loading">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  } else if (activities.length === 0) {
    if (providerConnected === null) {
      body = (
        <View className="px-4 py-3 items-start" testID="home-activities-loading">
          <ActivityIndicator color={colors.tokens.primary} />
        </View>
      );
    } else if (providerConnected) {
      body = <EmptyState testID="home-activities-empty">{t('home.activities.empty')}</EmptyState>;
    } else {
      body = (
        <EmptyState
          action={{ label: t('shell.connectBannerAction'), onPress: onConnect, testID: 'home-activities-connect' }}
          testID="home-activities-no-provider"
        >
          {t('home.activities.noProvider')}
        </EmptyState>
      );
    }
  } else {
    const [latest, ...older] = activities;
    body = (
      <View>
        <View testID="home-activity-latest">
          <LatestMap activity={latest} />
          <ActivityButton
            activity={latest}
            openDraft={openDraft}
            testID={`home-activity-${latest.provider}-${latest.id}`}
          />
        </View>
        {older.map((activity) => (
          <View key={`${activity.provider}:${activity.id}`} className="border-t border-border-faint">
            <ActivityButton
              activity={activity}
              openDraft={openDraft}
              leading={<ActivitySketch activity={activity} />}
              testID={`home-activity-${activity.provider}-${activity.id}`}
            />
          </View>
        ))}
      </View>
    );
  }

  return (
    <Section title={t('home.activities.heading')} testID="home-section-activities">
      {hasData ? <SyncLine stale={stale} staleRefetch={staleRefetch} asOf={asOf} /> : null}
      {body}
      {isError && hasData ? (
        <EmptyState
          action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-activities-retry' }}
          testID="home-activities-error"
        >
          {t('home.activities.loadFailed')}
        </EmptyState>
      ) : null}
    </Section>
  );
}
