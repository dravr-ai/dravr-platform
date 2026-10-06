// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's Today folded to one line above the conversation on a narrow screen — today's session, form, the latest route
// ABOUTME: A tap opens the whole of Today; it steps aside while the athlete reads back up the thread

import { useMemo } from 'react';
import { clsx } from 'clsx';
import { ChevronDown } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { planDayOn } from '@pierre/shared-types';
import { FORM_BAND_LABEL_KEY, formLine } from '@pierre/shared-constants';
import { decodePolyline } from '@pierre/domain-utils';
import { recentActivitiesQuery, useTrainingPlan, useTrainingStatus } from '../../hooks/useHome';
import { RouteSketch } from './RouteSketch';
import { planWindow } from './homeFormat';

interface TodayPeekProps {
  /** Open the whole of Today — a drawer on a tablet, a sheet on a phone. */
  onOpen: () => void;
  /**
   * Folded away: the athlete is reading back up the thread. It comes back at
   * the latest message (Phil, 2026-10-05).
   */
  hidden: boolean;
}

/** Today's session in a few words, or `null` while the plan says nothing about today. */
function useTodayHeadline(): string | null {
  const { t } = useTranslation();
  const plan = useTrainingPlan();
  if (plan.data === undefined || plan.data.plan === null) return null;
  const calendar = planWindow(plan.data.today);
  if (calendar === null) return null;
  const today = planDayOn(plan.data.plan, calendar.today);
  if (today.kind === 'rest') return t('chat.restDay');
  if (today.kind !== 'session') return null;
  return [today.day.workout, today.day.duration_min !== undefined ? `${today.day.duration_min} min` : null]
    .filter((part): part is string => typeof part === 'string' && part.length > 0)
    .join(' · ');
}

/** The form band and its share of the athlete's own fitness — the same words HomeStatus leads with. */
function useFormSummary(): string | null {
  const { t } = useTranslation();
  const status = useTrainingStatus();
  const form = status.data?.form ?? null;
  if (form === null) return null;
  const figure = formLine(t, form);
  return figure === null ? t(FORM_BAND_LABEL_KEY[form.band]) : `${t(FORM_BAND_LABEL_KEY[form.band])} · ${figure}`;
}

/**
 * One line that stands for Home's Today panel where there is no room for it
 * beside the thread. It reads the caches the panel fills; the recent
 * activities are read without the panel's refresh schedule, which the panel
 * runs itself once opened.
 */
export function TodayPeek({ onOpen, hidden }: TodayPeekProps) {
  const { t } = useTranslation();
  const headline = useTodayHeadline();
  const form = useFormSummary();
  const recent = useQuery(recentActivitiesQuery());
  const latest = recent.data?.activities[0] ?? null;
  const points = useMemo(
    () => (latest?.summary_polyline ? decodePolyline(latest.summary_polyline) : null),
    [latest?.summary_polyline],
  );

  return (
    <div
      className={clsx(
        'shrink-0 overflow-hidden px-3 transition-[max-height,opacity] duration-200 ease-out motion-reduce:transition-none',
        hidden ? 'max-h-0 opacity-0' : 'max-h-24 opacity-100',
      )}
      aria-hidden={hidden}
      inert={hidden}
    >
      <button
        type="button"
        onClick={onOpen}
        data-testid="home-today-peek"
        aria-label={t('home.personal.todayOpen')}
        className="my-2 flex w-full items-center gap-3 rounded-xl bg-surface-container-low py-2 pl-3 pr-2 text-left transition-colors hover:bg-surface-container focus-ring"
      >
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-baseline gap-1.5">
            <span className="shrink-0 text-xs text-on-surface-variant">{t('chat.dayToday')}</span>
            {headline !== null && (
              <span className="truncate text-sm font-semibold text-on-surface">{headline}</span>
            )}
          </span>
          {form !== null && <span className="mt-0.5 block truncate text-xs text-on-surface-variant">{form}</span>}
        </span>
        {points !== null && points.length > 1 && latest !== null && (
          <span className="shrink-0">
            <RouteSketch points={points} label={latest.name} compact />
          </span>
        )}
        <ChevronDown className="h-5 w-5 shrink-0 text-on-surface-variant" aria-hidden="true" />
      </button>
    </div>
  );
}
