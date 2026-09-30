// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home "Recent activities" section — the latest on a live map, the four before it with a route sketch
// ABOUTME: No GPS in the recording, a stale cache and a failed sync are each said in words; a tap opens a chat drafted about the activity

import React, { useEffect } from 'react';
import { AccessibilityInfo, ActivityIndicator, Platform, Pressable, Text, View } from 'react-native';
import type { HomeActivity, SyncFailure } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useActivityRoute } from '../../hooks/useHome';
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
 * The latest activity's map. A row that says `has_gps: false` had its route
 * read once and the recording held no GPS: it says so and costs no request.
 * Every other row asks the route endpoint — its route may never have been
 * read — and the answer is what says whether there is a track: a route to
 * draw, or the reason there is none. The route arrives privacy-trimmed from
 * the server and is drawn by the chat's own route card, loaded on demand
 * behind the boundary that keeps a runtime without MapLibre (Expo Go)
 * showing a sentence instead of losing the screen.
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
  // A read that failed, or one the server could not make just now
  // (`unavailable`), is the same sentence and the same retry: neither says
  // anything about whether the activity recorded a route. While a read is in
  // flight — the retry's included — the map says it is loading, so a retry
  // is seen to do something.
  if ((route.isError || route.reason === 'unavailable') && !route.isFetching) {
    return (
      <EmptyState
        action={{ label: t('common.retry'), onPress: route.retry, testID: 'home-latest-map-retry' }}
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
 * Where the list stands against the provider: checking while the server says
 * a refresh is running, otherwise when it last synced — the web page's rule.
 * After a failed sync that time is the failing provider's own last good one,
 * which the caller passes as `asOf`: the page says a time once.
 */
function SyncLine({ refreshing, asOf }: { refreshing: boolean; asOf: string | null }) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  if (refreshing) {
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

/**
 * The latest refresh of one provider failed: said plainly, naming it, with a
 * retry that asks the server to refresh it now. When it last synced well is
 * the sync line's to say, once.
 *
 * Read once when it appears: TalkBack reads the polite live region, and iOS,
 * which has no live region, has VoiceOver told in words. The retry is a
 * full-height button, not a word inside a line.
 */
function SyncFailed({ failure, onRetry }: { failure: SyncFailure; onRetry: () => void }) {
  const { t } = useTranslation();
  const message = `${failure.provider_name} · ${t('providers.syncFailed')}`;
  useEffect(() => {
    if (Platform.OS === 'ios') {
      AccessibilityInfo.announceForAccessibility(message);
    }
  }, [message, failure.failed_at]);
  return (
    <View
      className="mx-4 mb-2 flex-row flex-wrap items-center rounded-lg bg-error/15 px-3"
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      testID="home-activities-sync-failed"
    >
      <Text className="py-2 text-sm text-on-error-container">{message}</Text>
      <Pressable
        onPress={onRetry}
        accessibilityRole="button"
        className="ml-1 min-h-11 justify-center px-2"
        testID="home-activities-sync-retry"
      >
        <Text className="text-sm font-medium text-on-error-container underline">{t('common.retry')}</Text>
      </Pressable>
    </View>
  );
}

interface RecentActivitiesProps {
  activities: HomeActivity[];
  /** False until the first answer; stays false while a read is pending, paused offline, or failed. */
  hasData: boolean;
  isError: boolean;
  /**
   * Whether a provider refresh is running, as a read made on this visit said
   * (`stale: true`) and no follow-up has since settled; see `useRecentActivities`.
   */
  refreshing: boolean;
  asOf: string | null;
  /** The provider whose latest refresh failed after its own last good sync, or null. */
  syncFailure: SyncFailure | null;
  /** Read the list again, after a read of it failed. */
  onRetry: () => void;
  /** The athlete's retry after a failed sync: the server refreshes the failing provider now. */
  onRetrySync: () => void;
  /**
   * Whether any fitness provider is connected, from the provider status —
   * the activity list carries no such flag. `null` until the status answers.
   */
  providerConnected: boolean | null;
  /**
   * Whether a connected provider still syncs — one not flagged
   * `needs_reauth` — from the same status. `null` until it answers.
   */
  syncing: boolean | null;
  /** Leave for Connections, where a provider is connected. */
  onConnect: () => void;
  openDraft: OpenDraft;
}

/**
 * The newest five activities: the latest on its map, the rest as rows with a
 * sketch of their route. An empty list says why in one sentence — no
 * provider to read from, with the way to connect one, or nothing synced yet.
 *
 * A connection to reconnect is named once, by the shell's banner above the
 * page, and never again here. The rows the cache holds stay as they are.
 * With no rows and every connected provider flagged, the section is left
 * out: the empty sentence would promise a sync no connection can make, and
 * the only true thing left to say — reconnect — is the banner's. It stays
 * only while the server says a refresh is running, which for a flagged
 * scrape session is its retry, or while a read has failed, and then shows
 * just that.
 */
export function RecentActivities({
  activities,
  hasData,
  isError,
  refreshing,
  asOf,
  syncFailure,
  onRetry,
  onRetrySync,
  providerConnected,
  syncing,
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
      if (syncing === false) {
        // A failed read still shows its retry below.
        if (!refreshing && !isError) {
          return null;
        }
        body = null;
      } else {
        body = <EmptyState testID="home-activities-empty">{t('home.activities.empty')}</EmptyState>;
      }
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
      {hasData ? (
        <SyncLine refreshing={refreshing} asOf={syncFailure !== null ? syncFailure.last_synced_at : asOf} />
      ) : null}
      {hasData && syncFailure !== null && !refreshing ? (
        <SyncFailed failure={syncFailure} onRetry={onRetrySync} />
      ) : null}
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
