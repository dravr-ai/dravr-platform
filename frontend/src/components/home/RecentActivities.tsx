// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's recent activities — the latest on the chat's live map, the four before it as route sketches
// ABOUTME: A tap opens the activity's own view; no provider, no GPS, no rows and a failed sync are each said in words

import { useMemo, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import type { HomeActivity } from '@pierre/shared-types';
import { decodePolyline, type LatLon } from '@pierre/domain-utils';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { CONNECTIONS_ROUTE } from '../../constants/surfaceLayout';
import { useActivityRoute, useProviderConnection, useRecentActivities } from '../../hooks/useHome';
import { activityViewRoute } from '../activity/activityRoute';
import { ActivityMap } from './ActivityMap';
import { RouteSketch } from './RouteSketch';
import { ROW_DATE, activityFigures, formatInstant, formatSyncTime, sportLabel } from './homeFormat';

interface RecentActivitiesProps {
  /**
   * Dashboard route navigator, `tab[/subview]` — the connect prompt leaves
   * for the connections pane, and a tap on an activity opens its own view.
   */
  onNavigate: (route: string) => void;
}

/**
 * The route points a sketch can be drawn from without asking the server: the
 * activity's own summary polyline, decoded. `null` means there is none to
 * decode — the route endpoint is then the only source — while an empty list
 * means the polyline was there and did not decode, which draws nothing.
 */
function polylinePoints(activity: HomeActivity): LatLon[] | null {
  if (activity.summary_polyline === null) return null;
  return decodePolyline(activity.summary_polyline) ?? [];
}

/**
 * The text half of a row: the day and the name on the first line, the sport
 * and its figures on the second. Spans only — it sits inside a button.
 */
function ActivitySummary({ activity }: { activity: HomeActivity }) {
  const { t, language } = useTranslation();
  const sport = sportLabel(t, activity.sport_type);
  return (
    <span className="block min-w-0 flex-1">
      <span className="flex min-w-0 items-baseline gap-2">
        <span className="shrink-0 font-mono text-xs text-on-surface-variant">
          {formatInstant(activity.start_date, language, ROW_DATE)}
        </span>
        <span className="truncate text-sm font-medium text-on-surface">{activity.name || sport}</span>
      </span>
      <span className="mt-0.5 block truncate text-xs text-on-surface-variant">
        {sport}
        {activityFigures(t, activity, language).map((figure) => (
          <span key={figure}>
            {' · '}
            <span className="font-mono">{figure}</span>
          </span>
        ))}
      </span>
    </span>
  );
}

/** The newest activity: its map, then the row that opens its view. */
function LatestActivity({ activity, onOpen }: { activity: HomeActivity; onOpen: (activity: HomeActivity) => void }) {
  return (
    <li data-testid="home-activity-latest" className="border-b ghost-border-faint pb-2">
      <ActivityMap activity={activity} burst />
      <button
        type="button"
        onClick={() => onOpen(activity)}
        className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left transition-colors hover:bg-surface-container-low/60 focus-ring touch-target"
      >
        <ActivitySummary activity={activity} />
      </button>
    </li>
  );
}

/**
 * One of the four before the newest: a sketch beside the row. The sketch
 * comes from the summary polyline when the activity carries one, else from
 * the route endpoint's coordinates unless the activity says its route held
 * no GPS (`has_gps: false`); an answer without a route draws none.
 */
function ActivityRow({
  activity,
  sketchSlot,
  onOpen,
}: {
  activity: HomeActivity;
  /** Whether the list keeps a sketch column — false when no row may have a route. */
  sketchSlot: boolean;
  onOpen: (activity: HomeActivity) => void;
}) {
  const { t } = useTranslation();
  const decoded = useMemo(() => polylinePoints(activity), [activity]);
  const route = useActivityRoute(activity.provider, activity.id, decoded === null && activity.has_gps, {
    burst: true,
  });
  const points = decoded ?? route.data?.route?.coordinates ?? null;
  return (
    <li data-testid="home-activity-row" className="border-b ghost-border-faint last:border-0">
      <button
        type="button"
        onClick={() => onOpen(activity)}
        className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left transition-colors hover:bg-surface-container-low/60 focus-ring touch-target"
      >
        {/* The slot keeps every row's text on one column, sketch or not. */}
        {sketchSlot && (
          <span className="flex h-12 w-16 shrink-0 items-center justify-center" data-testid="home-sketch-slot">
            {points !== null && <RouteSketch points={points} label={t('home.activities.sketchAlt')} />}
          </span>
        )}
        <ActivitySummary activity={activity} />
      </button>
    </li>
  );
}

/**
 * The section: its heading and sync line, then the rows, the connect prompt
 * or the empty sentence — never a made-up row.
 *
 * A provider to reconnect is named by the app shell's reconnect banner, above
 * every tab, so this card does not say it a second time; the cached rows it
 * still shows are the athlete's own activities.
 */
export function RecentActivities({ onNavigate }: RecentActivitiesProps) {
  const { t, language } = useTranslation();
  const openActivity = (activity: HomeActivity) => onNavigate(activityViewRoute(activity.provider, activity.id));
  const recent = useRecentActivities();
  const providers = useProviderConnection();

  // One time on the card, once: while a refresh runs, that it is checking;
  // after a failed sync, when the provider that failed last synced well —
  // its rows are that old, whatever another provider did since; otherwise
  // the last sync across every provider. The failure itself is said in its
  // own line with its retry, and is not shown while a new attempt runs.
  const failure = recent.data?.sync_failure ?? null;
  const lastGood = failure !== null ? failure.last_synced_at : (recent.data?.as_of ?? null);
  const status = recent.refreshing
    ? t('home.activities.refreshing')
    : lastGood !== null
      ? t('home.activities.syncedAt', { time: formatSyncTime(lastGood, language) })
      : undefined;
  const syncFailed =
    failure !== null && !recent.refreshing ? (
      <p
        role="alert"
        data-testid="home-sync-failed"
        className="mb-2 rounded-lg bg-error/15 px-3 py-2 text-sm text-on-error-container"
      >
        {failure.provider_name} · {t('providers.syncFailed')}{' '}
        <button
          type="button"
          onClick={recent.retry}
          data-testid="home-sync-retry"
          className="rounded font-medium underline underline-offset-2 focus-ring touch-target"
        >
          {t('common.retry')}
        </button>
      </p>
    ) : null;
  const activities = recent.data?.activities ?? [];
  const noProvider = providers.loaded && !providers.connected;
  const connectPrompt = (
    <EmptyState
      data-testid="home-connect-provider"
      action={{ label: t('shell.connectBannerAction'), onClick: () => onNavigate(CONNECTIONS_ROUTE) }}
    >
      {t('home.activities.noProvider')}
    </EmptyState>
  );

  let body: ReactNode;
  if (recent.isPending) {
    body = (
      <div className="flex py-3" role="status" aria-label={t('common.loading')}>
        <div className="pierre-spinner" />
      </div>
    );
  } else if (recent.isError && recent.data === undefined) {
    body = (
      <EmptyState
        data-testid="home-activities-failed"
        action={{ label: t('common.retry'), onClick: recent.refetch }}
      >
        {t('home.activities.loadFailed')}
      </EmptyState>
    );
  } else if (activities.length === 0) {
    if (noProvider) {
      body = connectPrompt;
    } else {
      body = <EmptyState>{t('home.activities.empty')}</EmptyState>;
    }
  } else {
    const [latest, ...earlier] = activities;
    // A column no row can fill is only an indent: a list whose earlier rows
    // all say their route held no GPS keeps its text flush with the latest
    // row instead.
    const sketchSlot = earlier.some((activity) => activity.has_gps || activity.summary_polyline !== null);
    body = (
      <>
        {noProvider && connectPrompt}
        <ul className={clsx(noProvider && 'mt-2')}>
          <LatestActivity key={`${latest.provider}:${latest.id}`} activity={latest} onOpen={openActivity} />
          {earlier.map((activity) => (
            <ActivityRow
              key={`${activity.provider}:${activity.id}`}
              activity={activity}
              sketchSlot={sketchSlot}
              onOpen={openActivity}
            />
          ))}
        </ul>
      </>
    );
  }

  return (
    <Section title={t('home.activities.heading')} headingLevel={3} description={status} data-testid="home-activities">
      {syncFailed}
      {body}
    </Section>
  );
}
