// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Today's Getting started — five first steps, done, current or open, read from the server so web and phone agree
// ABOUTME: A row sends its flow's starter into the thread; Hide is a server preference, and the section folds then goes once done (carnet#1098)

import { useMemo, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { Check } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { Trans, useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CheckInCadence, GettingStartedRow } from '@pierre/shared-types';
import { formatDuration } from '@pierre/domain-utils';
import { athleteApi, coachesApi, providersApi, userApi } from '../../services/api';
import { recentActivitiesQuery } from '../../hooks/useHome';
import { useGettingStarted } from './gettingStartedState';
import { usePersonalConversation } from '../../hooks/useConversationList';
import { Section } from '../ui/Section';
import type { TodayActions } from '../ChatTab';

/** Each row's strings, keyed by the server's row id. */
const ROW_KEY: Record<string, string> = {
  connect: 'connect',
  meet_team: 'meetTeam',
  last_workout: 'lastWorkout',
  about_me_more: 'profile',
  check_in: 'checkIn',
};

const CHECK_IN_HINT: Record<CheckInCadence, string> = {
  daily: 'home.gettingStarted.rows.checkIn.hintDaily',
  three_per_week: 'home.gettingStarted.rows.checkIn.hintThreePerWeek',
  weekly: 'home.gettingStarted.rows.checkIn.hintWeekly',
  off: 'home.gettingStarted.rows.checkIn.hintOff',
};

/**
 * The supporting reads a row's hint names: the provider, the lead, the latest
 * activity, the profile, the cadence. Only read while the rows are on screen,
 * so an athlete whose Getting started is gone costs no extra request.
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
  const onboarding = useQuery({
    queryKey: QUERY_KEYS.user.onboardingStatus(),
    queryFn: () => userApi.getOnboardingStatus(),
    enabled,
  });
  const checkIn = useQuery({
    queryKey: QUERY_KEYS.user.checkIn(),
    queryFn: () => athleteApi.getCheckIn(),
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

  return (row) => {
    switch (row.id) {
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
        if (row.done || onboarding.data === undefined) return null;
        if (typeof onboarding.data.pillars_covered !== 'number' || typeof onboarding.data.pillars_total !== 'number') {
          return null;
        }
        return (
          <Trans
            i18nKey="home.gettingStarted.rows.profile.hint"
            values={{ covered: onboarding.data.pillars_covered, total: onboarding.data.pillars_total }}
            components={{ num: <span className="font-mono" /> }}
          />
        );
      case 'check_in':
        if (!row.eligible) return t('home.gettingStarted.rows.checkIn.needsData');
        if (row.done || checkIn.data === undefined) return null;
        if (checkIn.data.cadence === null || checkIn.data.cadence === 'off') return t(CHECK_IN_HINT.off);
        return time === null ? null : t(CHECK_IN_HINT[checkIn.data.cadence], { time });
      default:
        return null;
    }
  };
}

interface GettingStartedProps {
  actions: TodayActions;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
}

/**
 * Getting started (carnet#1098): connect your data, then the four first-run
 * flows (carnet#1099), in the server's order and with its verdicts — done,
 * current, and whether a row's prerequisite is met. A done row is plain text
 * with a check; the current row sits on the `surface-container-low` tint
 * with `aria-current`; an open row is an ink link. Nothing here is a filled
 * button: these are offers.
 *
 * A tap on row 1 goes to Settings › Connections; one on a flow sends its
 * starter postback (`gs:<pos>:<id>`) into the Home thread, the same send the
 * empty thread's starters make, so the server records it as a tap from this
 * surface (carnet#1100). Once the last flow is done the section folds to one
 * line naming the way back, and is gone seven days later.
 */
export function GettingStarted({ actions, onNavigate }: GettingStartedProps) {
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
          const actionable = !row.done && (row.id === 'connect' || (row.eligible && row.postback !== null));
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
