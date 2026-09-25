// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's recent activities — the latest on the chat's live map, the four before it as route sketches
// ABOUTME: A tap drafts "analyze my activity" in a new chat; no provider, no GPS and no rows are each said in words

import { useMemo, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import type { HomeActivity } from '@pierre/shared-types';
import { decodePolyline, type LatLon } from '@pierre/domain-utils';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import RouteView from '../chat/RouteView';
import { CONNECTIONS_ROUTE } from '../../constants/surfaceLayout';
import { useActivityRoute, useProviderConnection, useRecentActivities } from '../../hooks/useHome';
import { RouteSketch } from './RouteSketch';
import {
  DRAFT_DATE,
  ROW_DATE,
  activityFigures,
  formatInstant,
  formatSyncTime,
  sportLabel,
} from './homeFormat';

interface RecentActivitiesProps {
  /** Dashboard route navigator, `tab[/subview]` — the connect prompt leaves for the connections pane. */
  onNavigate: (route: string) => void;
  /** Open a new chat whose composer holds `text`. */
  onOpenChatDraft: (text: string) => void;
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

/** The draft a tap on an activity puts in the composer. */
function useAnalyzeDraft(activity: HomeActivity): string {
  const { t, language } = useTranslation();
  return t('home.activities.analyzeDraft', {
    date: formatInstant(activity.start_date, language, DRAFT_DATE),
    sport: sportLabel(t, activity.sport_type),
  });
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
        {activityFigures(activity).map((figure) => (
          <span key={figure}>
            {' · '}
            <span className="font-mono">{figure}</span>
          </span>
        ))}
      </span>
    </span>
  );
}

/** The frame the map fills, holding a line of text while there is no map in it. */
function MapNote({ children }: { children: ReactNode }) {
  return (
    <div className="my-4 flex h-64 w-full items-center justify-center rounded-[10px] border ghost-border bg-surface-container-lowest px-4 text-center text-sm text-on-surface-variant sm:h-80">
      {children}
    </div>
  );
}

/**
 * The latest activity's map. A track the activity never recorded costs no
 * request; everything else asks the route endpoint once and draws what it
 * answers with the chat's own map component, unchanged.
 */
function LatestMap({ activity }: { activity: HomeActivity }) {
  const { t } = useTranslation();
  const route = useActivityRoute(activity.provider, activity.id, activity.has_gps);

  if (!activity.has_gps) {
    return <p className="py-3 text-sm text-on-surface-variant">{t('chat.routeNoTrack')}</p>;
  }
  if (route.data === undefined) {
    return route.isError ? (
      <EmptyState
        data-testid="home-route-failed"
        action={{ label: t('common.retry'), onClick: () => void route.refetch() }}
      >
        {t('home.activities.routeFailed')}
      </EmptyState>
    ) : (
      <MapNote>
        <span role="status">{t('home.activities.mapLoading')}</span>
      </MapNote>
    );
  }
  if (route.data.route !== null) {
    return <RouteView view={route.data.route} />;
  }
  return (
    <p className="py-3 text-sm text-on-surface-variant">
      {route.data.reason === 'too_short' ? t('home.activities.routeTooShort') : t('chat.routeNoTrack')}
    </p>
  );
}

/** The newest activity: its map, then the row that opens it in chat. */
function LatestActivity({ activity, onOpenChatDraft }: { activity: HomeActivity; onOpenChatDraft: (text: string) => void }) {
  const draft = useAnalyzeDraft(activity);
  return (
    <li data-testid="home-activity-latest" className="border-b ghost-border-faint pb-2">
      <LatestMap activity={activity} />
      <button
        type="button"
        onClick={() => onOpenChatDraft(draft)}
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
 * the route endpoint's coordinates when it recorded GPS, else there is none.
 */
function ActivityRow({
  activity,
  sketchSlot,
  onOpenChatDraft,
}: {
  activity: HomeActivity;
  /** Whether the list keeps a sketch column — false when no row can draw one. */
  sketchSlot: boolean;
  onOpenChatDraft: (text: string) => void;
}) {
  const { t } = useTranslation();
  const draft = useAnalyzeDraft(activity);
  const decoded = useMemo(() => polylinePoints(activity), [activity]);
  const route = useActivityRoute(activity.provider, activity.id, decoded === null && activity.has_gps);
  const points = decoded ?? route.data?.route?.coordinates ?? null;
  return (
    <li data-testid="home-activity-row" className="border-b ghost-border-faint last:border-0">
      <button
        type="button"
        onClick={() => onOpenChatDraft(draft)}
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
 */
export function RecentActivities({ onNavigate, onOpenChatDraft }: RecentActivitiesProps) {
  const { t, language } = useTranslation();
  const recent = useRecentActivities();
  const providers = useProviderConnection();

  const asOf = recent.data?.as_of ?? null;
  const status = recent.refreshing
    ? t('home.activities.refreshing')
    : asOf !== null
      ? t('home.activities.syncedAt', { time: formatSyncTime(asOf, language) })
      : undefined;
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
    body = noProvider ? connectPrompt : <EmptyState>{t('home.activities.empty')}</EmptyState>;
  } else {
    const [latest, ...earlier] = activities;
    // A column no row can fill is only an indent: an indoor-only list keeps
    // its text flush with the latest row instead.
    const sketchSlot = earlier.some((activity) => activity.has_gps || activity.summary_polyline !== null);
    body = (
      <>
        {noProvider && connectPrompt}
        <ul className={clsx(noProvider && 'mt-2')}>
          <LatestActivity key={`${latest.provider}:${latest.id}`} activity={latest} onOpenChatDraft={onOpenChatDraft} />
          {earlier.map((activity) => (
            <ActivityRow
              key={`${activity.provider}:${activity.id}`}
              activity={activity}
              sketchSlot={sketchSlot}
              onOpenChatDraft={onOpenChatDraft}
            />
          ))}
        </ul>
      </>
    );
  }

  return (
    <Section title={t('home.activities.heading')} headingLevel={3} description={status} data-testid="home-activities">
      {body}
    </Section>
  );
}
