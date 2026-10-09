// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home "Recent activities" section — the latest on a live map, the four before it with a route sketch
// ABOUTME: A .fit upload sits by the title; a fetch heads the list as its own row; no GPS and a failed sync are said in words

import React, { useEffect, useState } from 'react';
import { AccessibilityInfo, ActivityIndicator, Platform, Pressable, Text, View } from 'react-native';
import type { HomeActivity, SyncFailure } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { useDistanceUnit, type RecentActivitiesSync, type UseActivityUploadResult } from '@pierre/ui-logic';
import { EmptyState, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { ActivityMap } from './ActivityMap';
import { ActivitySketch, SKETCH_SIZE } from './RouteSketch';
import { UploadActivityAction, UploadActivityStatus } from './UploadActivity';
import { activityFigures, instantShortDate, sportLabel, syncedAtLabel } from './homeFormat';

type OpenActivity = (activity: HomeActivity) => void;

/** "Sat 20 Sep · Ride · 92.0 km · 3h 41m 5s" (or "57.2 mi") — the row's second line, its figures in mono. */
function ActivityFacts({ activity }: { activity: HomeActivity }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  return (
    <Text className="text-sm text-text-secondary" numberOfLines={1}>
      <Text className="font-mono tabular-nums">{instantShortDate(activity.start_date, language)}</Text>
      {' · '}
      {sportLabel(t, activity.sport_type)}
      {/* The attribution a Garmin-recorded activity carries, as served (carnet#521). */}
      {activity.attribution && <Text testID="activity-attribution">{` · ${activity.attribution}`}</Text>}
      {activityFigures(t, activity, language, unit).map((figure, index) => (
        // Positional: the figures are distance, time and climb, in that order.
        <Text key={index}>
          {' · '}
          <Text className="font-mono tabular-nums">{figure}</Text>
        </Text>
      ))}
    </Text>
  );
}

/** One activity's words, pressable into its own view. */
function ActivityButton({
  activity,
  openActivity,
  leading,
  testID,
}: {
  activity: HomeActivity;
  openActivity: OpenActivity;
  leading?: React.ReactNode;
  testID: string;
}) {
  return (
    <Pressable
      onPress={() => openActivity(activity)}
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
 * When the list last synced with the provider. After a failed sync that time
 * is the failing provider's own last good one, which the caller passes as
 * `asOf`: the page says a time once. A fetch in progress is said by the
 * list's own top row ({@link FetchingRow}), not here.
 */
function SyncLine({ asOf }: { asOf: string | null }) {
  const { t, language } = useTranslation();
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
 * Whether the athlete asked the system to reduce motion, kept current while
 * the screen is open. False until the setting is read.
 */
function useReduceMotion(): boolean {
  const [reduce, setReduce] = useState(false);
  useEffect(() => {
    let active = true;
    void AccessibilityInfo.isReduceMotionEnabled().then((enabled) => {
      if (active) {
        setReduce(enabled);
      }
    });
    const subscription = AccessibilityInfo.addEventListener('reduceMotionChanged', setReduce);
    return () => {
      active = false;
      subscription.remove();
    };
  }, []);
  return reduce;
}

/**
 * The row at the top of the list while new activities are read from a
 * provider, shaped like an activity row — a sketch-sized slot holding a
 * spinner, the sentence over a placeholder second line — so the rows that
 * land take its place without the list jumping.
 *
 * A polite live region for TalkBack; iOS, which has none, has VoiceOver told
 * in words when it appears. With reduced motion asked for, the slot holds
 * still: a plain tile instead of the spinner.
 */
function FetchingRow({ separated }: { separated: boolean }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const reduceMotion = useReduceMotion();
  const message = t('home.activities.fetching');
  useEffect(() => {
    if (Platform.OS === 'ios') {
      AccessibilityInfo.announceForAccessibility(message);
    }
  }, [message]);
  return (
    <View
      role="status"
      accessibilityLiveRegion="polite"
      accessible
      accessibilityLabel={message}
      className={`flex-row items-center px-4 py-2 min-h-11 ${separated ? 'border-b border-border-faint' : ''}`}
      testID="home-activities-fetching"
    >
      <View
        className="items-center justify-center rounded-lg bg-surface-container-low"
        style={{ width: SKETCH_SIZE, height: SKETCH_SIZE }}
        testID={reduceMotion ? 'home-activities-fetching-still' : 'home-activities-fetching-spinner'}
      >
        {reduceMotion ? null : <ActivityIndicator size="small" color={colors.tokens.primary} />}
      </View>
      <View className="ml-3 flex-1 min-w-0">
        <Text className="text-base text-text-primary" numberOfLines={1}>
          {message}
        </Text>
        <View className="mt-1.5 h-2.5 w-32 rounded bg-surface-container-high" />
      </View>
    </View>
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
   * Fetching, failed or settled — the one sync state the list shows, from
   * `useRecentActivities`: a refresh a read made on this visit reported
   * (`stale: true`) and no follow-up has since settled, or the athlete's
   * retry in flight, is `fetching`.
   */
  sync: RecentActivitiesSync;
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
  /** The `.fit` upload the section's title offers, and how the last one went. */
  uploader: UseActivityUploadResult;
  /** Open the tapped activity's own view. */
  openActivity: OpenActivity;
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
 * only while new activities are fetched — a refresh the server says is
 * running, which for a flagged scrape session is its retry, or the
 * athlete's own Retry — or while a read has failed, and then shows just that.
 *
 * While new activities are fetched the list is headed by a row that says so;
 * the rows that land replace it. It and the failed sync are never shown
 * together: the failure is said once nothing is running.
 */
export function RecentActivities({
  activities,
  hasData,
  isError,
  sync,
  asOf,
  syncFailure,
  onRetry,
  onRetrySync,
  providerConnected,
  syncing,
  onConnect,
  openActivity,
  uploader,
}: RecentActivitiesProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const fetching = sync === 'fetching';

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
      if (fetching) {
        // The first activities are on their way: the row that says so, not a
        // sentence that says there are none.
        body = <FetchingRow separated={false} />;
      } else if (syncing === false) {
        // Nothing to list and nothing on its way: the section keeps only its
        // title and the Upload action (carnet#818), so an athlete with no
        // synced workout can still add one; a failed read shows its retry.
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
        {fetching ? <FetchingRow separated /> : null}
        <View testID="home-activity-latest">
          <ActivityMap activity={latest} testIDPrefix="home-latest" burst />
          <ActivityButton
            activity={latest}
            openActivity={openActivity}
            testID={`home-activity-${latest.provider}-${latest.id}`}
          />
        </View>
        {older.map((activity) => (
          <View key={`${activity.provider}:${activity.id}`} className="border-t border-border-faint">
            <ActivityButton
              activity={activity}
              openActivity={openActivity}
              leading={<ActivitySketch activity={activity} />}
              testID={`home-activity-${activity.provider}-${activity.id}`}
            />
          </View>
        ))}
      </View>
    );
  }

  return (
    <Section
      title={t('home.activities.heading')}
      actions={<UploadActivityAction uploader={uploader} />}
      testID="home-section-activities"
    >
      <UploadActivityStatus uploader={uploader} />
      {hasData ? <SyncLine asOf={syncFailure !== null ? syncFailure.last_synced_at : asOf} /> : null}
      {hasData && syncFailure !== null && sync === 'failed' ? (
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
