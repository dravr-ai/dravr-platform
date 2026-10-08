// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's week strip expanded to a month — a sheet with the month's grid: workouts done on past days, the plan's on the rest
// ABOUTME: Pages by month between stored history and the plan's last week; a tapped day opens the same detail the strip shows

import { useState } from 'react';
import { clsx } from 'clsx';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { CalendarDay, WorkoutPlan } from '@pierre/shared-types';
import { calendarDay, firstOfMonth, lastMonday, monthGrid, shiftMonth } from '@pierre/shared-types';
import { Sheet } from '../ui/Sheet';
import { EmptyState } from '../ui/EmptyState';
import { useHomeCalendar } from '../../hooks/useHomeCalendar';
import { CalendarDayDetail } from './CalendarDayDetail';
import { calendarDayLabel } from './calendarFormat';
import { DRAFT_DATE, formatCivilDate } from './homeFormat';

interface HomeMonthSheetProps {
  plan: WorkoutPlan | null;
  today: string;
  /** A day of the month the sheet opens on: the week the strip shows. */
  startMonth: string;
  onClose: () => void;
  onOpenChatDraft: (text: string) => void;
  onNavigate: (route: string) => void;
}

const DETAIL_ID = 'home-month-detail';

/**
 * The marks under a day's number: a filled dot for a workout done, a hollow
 * ring for a planned session — filled against hollow, so the difference
 * survives any colour vision; the cell's spoken label says it in words.
 */
function Marks({ day, today }: { day: CalendarDay; today: string }) {
  const done = day.activities.length > 0;
  const planned = day.plan?.kind === 'session' && (day.date >= today || !done);
  return (
    <span className="flex h-2 items-center justify-center gap-0.5" aria-hidden="true">
      {done && <span data-testid={`home-month-done-${day.date}`} className="h-1.5 w-1.5 rounded-full bg-primary" />}
      {planned && (
        <span data-testid={`home-month-planned-${day.date}`} className="h-1.5 w-1.5 rounded-full border border-primary" />
      )}
    </span>
  );
}

/**
 * The month sheet. It opens on the month of the week the strip shows and
 * reads that month's whole-week grid in one calendar read; a day outside the
 * month is drawn faint, a day older than stored history fainter still, and
 * the sentence under the grid says where history starts rather than leave
 * those days to read as rest.
 */
export function HomeMonthSheet({
  plan,
  today,
  startMonth,
  onClose,
  onOpenChatDraft,
  onNavigate,
}: HomeMonthSheetProps) {
  const { t, language } = useTranslation();
  const [month, setMonth] = useState(firstOfMonth(startMonth));
  const [selected, setSelected] = useState<string | null>(null);
  const grid = monthGrid(month);
  const first = grid[0][0];
  const last = grid[grid.length - 1][6];
  const calendar = useHomeCalendar(first, last);
  const answer = calendar.isPlaceholderData ? undefined : calendar.data;
  const historyStart = calendar.data?.history_start ?? null;
  const open = selected !== null && answer !== undefined ? calendarDay(answer, selected) : null;
  const title = formatCivilDate(month, language, { month: 'long', year: 'numeric' });
  const arrow =
    'flex h-11 w-11 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring disabled:opacity-40 disabled:hover:bg-transparent';

  const page = (months: number) => {
    setMonth((current) => shiftMonth(current, months));
    setSelected(null);
  };

  return (
    <Sheet
      side="bottom"
      title={title}
      onClose={onClose}
      data-testid="home-month-sheet"
      actions={
        <>
          <button
            type="button"
            data-testid="home-month-previous"
            aria-label={t('home.calendar.previousMonth')}
            title={t('home.calendar.previousMonth')}
            disabled={historyStart !== null && month <= historyStart}
            onClick={() => page(-1)}
            className={arrow}
          >
            <ChevronLeft aria-hidden="true" className="h-5 w-5" />
          </button>
          <button
            type="button"
            data-testid="home-month-next"
            aria-label={t('home.calendar.nextMonth')}
            title={t('home.calendar.nextMonth')}
            disabled={shiftMonth(month, 1) > lastMonday(plan, today)}
            onClick={() => page(1)}
            className={arrow}
          >
            <ChevronRight aria-hidden="true" className="h-5 w-5" />
          </button>
        </>
      }
    >
      <div className="mx-auto max-w-[560px] px-4 pb-4">
        <div className="grid grid-cols-7 gap-1 pb-1" aria-hidden="true">
          {grid[0].map((date) => (
            <span key={date} className="text-center text-xs text-on-surface-variant">
              {formatCivilDate(date, language, { weekday: 'narrow' })}
            </span>
          ))}
        </div>
        {calendar.isError && answer === undefined ? (
          <EmptyState
            data-testid="home-month-failed"
            action={{ label: t('common.retry'), onClick: () => void calendar.refetch() }}
          >
            {t('home.calendar.loadFailed')}
          </EmptyState>
        ) : (
          <ol className="grid grid-cols-7 gap-1" aria-busy={answer === undefined} data-testid="home-month-grid">
            {grid.flat().map((date) => {
              const day = answer === undefined ? null : calendarDay(answer, date);
              const inMonth = date.slice(0, 7) === month.slice(0, 7);
              const isToday = date === today;
              const isSelected = date === selected;
              return (
                <li key={date} className="min-w-0">
                  <button
                    type="button"
                    data-testid={`home-month-day-${date}`}
                    aria-expanded={isSelected}
                    aria-controls={isSelected ? DETAIL_ID : undefined}
                    aria-current={isToday ? 'date' : undefined}
                    aria-label={calendarDayLabel(t, formatCivilDate(date, language, DRAFT_DATE), day, today)}
                    disabled={day === null}
                    onClick={() => setSelected((current) => (current === date ? null : date))}
                    className={clsx(
                      'flex w-full flex-col items-center gap-0.5 rounded-lg border py-1.5 transition-colors focus-ring touch-target',
                      isToday
                        ? 'bg-primary-container text-on-primary-container'
                        : 'bg-surface-container-lowest text-on-surface hover:bg-surface-container-low/60',
                      isSelected ? 'border-primary' : isToday ? 'border-transparent' : 'ghost-border',
                      !inMonth && 'opacity-60',
                      day?.beforeHistory === true && 'opacity-30',
                    )}
                  >
                    <span className="font-mono text-sm">{formatCivilDate(date, language, { day: 'numeric' })}</span>
                    {day !== null && !day.beforeHistory ? <Marks day={day} today={today} /> : <span className="h-2" />}
                  </button>
                </li>
              );
            })}
          </ol>
        )}
        {open !== null && (
          <CalendarDayDetail
            day={open}
            today={today}
            id={DETAIL_ID}
            onOpenChatDraft={onOpenChatDraft}
            onNavigate={onNavigate}
          />
        )}
        {answer !== undefined && historyStart !== null && first <= historyStart && (
          <p data-testid="home-month-history-start" className="mt-3 text-sm text-on-surface-variant">
            {t('home.calendar.historyStart', { date: formatCivilDate(historyStart, language, DRAFT_DATE) })}
          </p>
        )}
        {answer !== undefined && answer.plan_weeks === null && (
          <p data-testid="home-month-no-plan" className="mt-3 text-sm text-on-surface-variant">
            {t('home.calendar.noPlan')}
          </p>
        )}
      </div>
    </Sheet>
  );
}
