// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Today's Getting started — the first steps, done, current or open, read from the server so web and phone agree
// ABOUTME: A row opens its sheet, Settings, or sends its flow's starter into the thread; Hide is a server preference, and the section folds then goes once done (carnet#1098)

import { useMemo, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { Check } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CheckInCadence, GettingStartedRow } from '@pierre/shared-types';
import { formatDuration } from '@pierre/domain-utils';
import { athleteApi, coachesApi, messagingLinkApi, providersApi } from '../../services/api';
import { recentActivitiesQuery } from '../../hooks/useHome';
import { useAvailableChannels } from '../../hooks/useAvailableChannels';
import { useGettingStarted } from './gettingStartedState';
import { usePersonalConversation } from '../../hooks/useConversationList';
import { Section } from '../ui/Section';
import type { TodayActions } from './personalToday';
import type { FirstStep, FirstStepRequest } from './FirstStepSheet';

/** Each row's strings, keyed by the server's row id. */
const ROW_KEY: Record<string, string> = {
  health_check: 'healthCheck',
  about_me_more: 'goalWhy',
  check_in: 'checkIn',
  chat_app: 'chatApp',
  connect: 'connect',
  meet_team: 'meetTeam',
  last_workout: 'lastWorkout',
};

/**
 * The rows that complete in a small sheet over the page (carnet#1530): the
 * health check put off at onboarding, the goal and the why, the check-in
 * cadence — each once a screen of the wizard.
 */
const SHEET_OF_ROW: Record<string, FirstStep> = {
  health_check: 'health_check',
  about_me_more: 'goal_why',
  check_in: 'check_in',
};

const CHECK_IN_HINT: Record<Exclude<CheckInCadence, 'off'>, string> = {
  daily: 'home.gettingStarted.rows.checkIn.hintDaily',
  three_per_week: 'home.gettingStarted.rows.checkIn.hintThreePerWeek',
  weekly: 'home.gettingStarted.rows.checkIn.hintWeekly',
};

/**
 * The supporting reads a row's hint names: the provider, the lead, the latest
 * activity, the profile, the cadence, the chat apps. Only read while the rows
 * are on screen, so an athlete whose Getting started is gone costs no extra
 * request.
 */
function useRowHints(enabled: boolean): (row: GettingStartedRow) => ReactNode {
  const { t, language } = useTranslation();
  const providers = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
    enabled,
  });
  const personal = usePersonalConversation();
  const agents = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    enabled,
    staleTime: 5 * 60_000,
  });
  const recent = useQuery({ ...recentActivitiesQuery(), enabled });
  const checkIn = useQuery({
    queryKey: QUERY_KEYS.user.checkIn(),
    queryFn: () => athleteApi.getCheckIn(),
    enabled,
  });
  // The chat-app sheet and Settings read the same two entries.
  const { channels } = useAvailableChannels({ enabled });
  const links = useQuery({
    queryKey: ['messaging-links'],
    queryFn: () => messagingLinkApi.listLinks(),
    enabled,
  });

  const provider = providers.data?.providers.find((p) => p.connected)?.display_name ?? null;
  const lead = agents.data?.agents.find((agent) => agent.id === personal.agentId)?.title ?? null;
  const latest = recent.data?.activities[0] ?? null;
  const localHour = checkIn.data?.local_hour;
  const time = useMemo(() => {
    if (localHour === undefined) return null;
    const at = new Date(2000, 0, 1, localHour, 0);
    return new Intl.DateTimeFormat(language, { hour: 'numeric', minute: '2-digit' }).format(at);
  }, [language, localHour]);
  // « Telegram ou WhatsApp »: the apps on offer, in the reader's own "or".
  const apps = useMemo(
    () =>
      channels.length === 0
        ? null
        : new Intl.ListFormat(language, { type: 'disjunction' }).format(channels.map((c) => c.display_name)),
    [channels, language],
  );
  const linkedChannel = Array.isArray(links.data) ? links.data[0]?.channel : undefined;
  const linkedApp =
    linkedChannel === undefined
      ? null
      : (channels.find((c) => c.channel === linkedChannel)?.display_name ?? null);

  return (row) => {
    switch (row.id) {
      case 'health_check':
        return row.done ? null : t('home.gettingStarted.rows.healthCheck.hint');
      case 'connect':
        if (row.done) return provider ? t('home.gettingStarted.rows.connect.doneHint', { provider }) : null;
        return t('home.gettingStarted.rows.connect.hint');
      case 'meet_team':
        return row.done || lead === null ? null : t('home.gettingStarted.rows.meetTeam.hint', { lead });
      case 'last_workout':
        if (!row.eligible) return t('home.gettingStarted.rows.lastWorkout.needsActivity');
        if (row.done || latest === null) return null;
        return `${latest.name} · ${formatDuration(t, Math.round(latest.duration_seconds / 60) * 60)}`;
      case 'about_me_more':
        return row.done ? null : t('home.gettingStarted.rows.goalWhy.hint');
      case 'check_in':
        // Open, the row says what it sets; answered, the rhythm it set.
        if (!row.done) return t('home.gettingStarted.rows.checkIn.hint');
        if (checkIn.data === undefined || checkIn.data.cadence === null || checkIn.data.cadence === 'off') return null;
        return time === null ? null : t(CHECK_IN_HINT[checkIn.data.cadence], { time });
      case 'chat_app':
        if (row.done) return linkedApp === null ? null : t('home.gettingStarted.rows.chatApp.doneHint', { app: linkedApp });
        return apps;
      default:
        return null;
    }
  };
}

interface GettingStartedProps {
  actions: TodayActions;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /** Open the chat-app sheet over the page — the « Écris-moi aussi ailleurs » row. */
  onOpenChatApp: () => void;
  /** Open a first step's sheet over the page — the health check, the goal and the why, the cadence. */
  onOpenFirstStep: (request: FirstStepRequest) => void;
}

/**
 * Getting started (carnet#1098), in the server's order and with its verdicts
 * — done, current, and whether a row's prerequisite is met. A done row is
 * plain text with a check; the current row sits on the
 * `surface-container-low` tint with `aria-current`; an open row is an ink
 * link. Nothing here is a filled button: these are offers.
 *
 * "Connect your data" comes first and goes to Settings › Connections. Then
 * the questions the onboarding wizard no longer asks, each completing in a
 * small sheet over the page (carnet#1530): the health check when it was put
 * off, the goal and the why, the check-in cadence. The chat-app row, listed
 * by the server only when the workspace offers an app and the list is still
 * open, opens the chat-app sheet (carnet#1525). Then the first-run flows,
 * whose tap sends the starter postback (`gs:<pos>:<id>`)
 * into the Home thread, the same send the empty thread's starters make, so
 * the server records it as a tap from this surface (carnet#1100). Closing a
 * sheet ticks nothing; once the last row is done the section folds to one
 * line naming the way back, and is gone seven days later.
 */
export function GettingStarted({ actions, onNavigate, onOpenChatApp, onOpenFirstStep }: GettingStartedProps) {
  const { t } = useTranslation();
  const { query, home, state } = useGettingStarted();
  const hintFor = useRowHints(state === 'open');

  if (state === null || state === 'gone' || query.data === undefined) return null;
  const data = query.data;

  if (state === 'folded') {
    return (
      <p data-testid="home-getting-started-folded" className="text-sm text-on-surface-variant">
        {t('home.gettingStarted.folded')}
      </p>
    );
  }

  const titleOf = (row: GettingStartedRow) => t(`home.gettingStarted.rows.${ROW_KEY[row.id]}.title`);

  const open = (row: GettingStartedRow) => {
    if (row.id === 'connect') {
      onNavigate('settings/connections');
      return;
    }
    if (row.id === 'chat_app') {
      onOpenChatApp();
      return;
    }
    const sheet = SHEET_OF_ROW[row.id];
    if (sheet) {
      onOpenFirstStep({ step: sheet });
      return;
    }
    if (row.postback) actions.send(row.postback, titleOf(row));
  };

  const title = (
    <>
      {t('home.gettingStarted.title')}{' '}
      <span aria-hidden="true" className="font-mono text-xs font-normal text-outline" data-testid="home-getting-started-count">
        {t('home.gettingStarted.count', { done: data.done_count, total: data.total })}
      </span>
      <span className="sr-only">{t('home.gettingStarted.countAria', { done: data.done_count, total: data.total })}</span>
    </>
  );
  const hide = (
    <button
      type="button"
      data-testid="home-getting-started-hide"
      className="rounded text-xs font-medium text-primary hover:underline focus-ring"
      aria-label={t('home.gettingStarted.hideAria')}
      onClick={() => home.update({ getting_started_hidden: true })}
    >
      {t('home.gettingStarted.hide')}
    </button>
  );

  const rows = data.rows.filter((row) => row.id in ROW_KEY).sort((a, b) => a.order - b.order);

  return (
    <Section title={title} headingLevel={3} actions={hide} data-testid="home-getting-started">
      <ol className="-mx-2 space-y-0.5">
        {rows.map((row) => {
          const hint = hintFor(row);
          const label = titleOf(row);
          const circle = (
            <span
              aria-hidden="true"
              className={clsx(
                'flex h-5 w-5 shrink-0 items-center justify-center rounded-full',
                row.done && 'bg-primary text-on-primary',
                !row.done && row.current && 'border-2 border-primary',
                !row.done && !row.current && 'border-[1.5px] border-outline',
              )}
            >
              {row.done && <Check className="h-3 w-3" strokeWidth={3} />}
            </span>
          );
          const text = (
            <span className="min-w-0 flex-1">
              <span
                className={clsx(
                  'block truncate text-sm',
                  row.done && 'font-normal text-on-surface-variant',
                  !row.done && row.current && 'font-semibold text-on-surface',
                  !row.done && !row.current && 'font-medium text-on-surface',
                )}
              >
                {label}
                {row.done && <span className="sr-only"> — {t('home.gettingStarted.doneSr')}</span>}
              </span>
              {hint !== null && <span className="block truncate text-xs text-outline">{hint}</span>}
            </span>
          );
          const rowClass = clsx(
            'flex w-full items-center gap-3 rounded-lg px-2 py-1 text-left',
            row.current && !row.done && 'bg-surface-container-low',
          );
          // A done row, or one whose prerequisite is missing, is not a link.
          const opensItsOwnPlace = row.id === 'connect' || row.id === 'chat_app' || row.id in SHEET_OF_ROW;
          const actionable = !row.done && (opensItsOwnPlace || (row.eligible && row.postback !== null));
          return (
            <li
              key={row.id}
              data-testid={`home-getting-started-row-${row.id}`}
              data-state={row.done ? 'done' : row.current ? 'current' : 'open'}
              aria-current={row.current && !row.done ? 'step' : undefined}
            >
              {actionable ? (
                <button
                  type="button"
                  className={clsx(rowClass, 'transition-colors hover:bg-surface-container-low focus-ring')}
                  onClick={() => open(row)}
                >
                  {circle}
                  {text}
                </button>
              ) : (
                <div className={rowClass}>
                  {circle}
                  {text}
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </Section>
  );
}
