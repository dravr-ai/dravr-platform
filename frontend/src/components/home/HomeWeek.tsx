// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's week — a Monday-to-Sunday strip that pages to past and next weeks and expands to a month sheet
// ABOUTME: Past days show the workouts done, future days the plan's sessions; it renders without a plan and stops at stored history

import { useState } from 'react';
import { clsx } from 'clsx';
import { Check, ChevronLeft, ChevronRight, CalendarDays } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import type { CalendarDay, PlanDayLookup, WorkoutPlan } from '@pierre/shared-types';
import { activityMinutes, addCivilDays, calendarDay, lastMonday, mondayOf, weekDays } from '@pierre/shared-types';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { WeekHeading } from '../chat/WorkoutPlanCard';
import { useTrainingPlan } from '../../hooks/useHome';
import { useHomeCalendar } from '../../hooks/useHomeCalendar';
import { CalendarDayDetail } from './CalendarDayDetail';
import { calendarDayLabel } from './calendarFormat';
import { HomeMonthSheet } from './HomeMonthSheet';
import { DRAFT_DATE, formatCivilDate, sportLabel } from './homeFormat';

interface HomeWeekProps {
  onOpenChatDraft: (text: string) => void;
  /** Dashboard route navigator: a workout on a day opens its own view. */
  onNavigate: (route: string) => void;
}

const DETAIL_ID = 'home-week-detail';
const DAYS_PER_WEEK = 7;

/** The few characters the plan's day leaves in a cell: the minutes, else the sport; rest is the word. */
function planHint(t: TFunction, lookup: PlanDayLookup | null): string {
  if (lookup === null) return '';
  if (lookup.kind === 'rest') return t('chat.restDay');
  if (lookup.kind === 'uncovered') return '—';
  if (lookup.day.duration_min !== undefined) return `${lookup.day.duration_min} min`;
  return typeof lookup.day.sport === 'string' ? sportLabel(t, lookup.day.sport) : '';
}

/** One day of the strip: weekday, date, and what was done on it or what the plan holds. */
function DayCell({
  date,
  day,
  today,
  isSelected,
  onToggle,
}: {
  date: string;
  /** Null while the week's answer is still on its way. */
  day: CalendarDay | null;
  today: string;
  isSelected: boolean;
  onToggle: (date: string) => void;
}) {
  const { t, language } = useTranslation();
  const isToday = date === today;
  const done = day !== null && day.activities.length > 0;
  const hint = day === null || day.beforeHistory ? '' : done ? `${activityMinutes(day.activities)} min` : planHint(t, day.plan);
  return (
    <li className="min-w-0">
      <button
        type="button"
        data-testid={`home-week-day-${date}`}
        aria-expanded={isSelected}
        aria-controls={isSelected ? DETAIL_ID : undefined}
        aria-current={isToday ? 'date' : undefined}
        aria-label={calendarDayLabel(t, formatCivilDate(date, language, DRAFT_DATE), day, today)}
        disabled={day === null}
        onClick={() => onToggle(date)}
        className={clsx(
          'flex w-full flex-col items-center gap-0.5 rounded-lg border px-0.5 py-1.5 transition-colors focus-ring touch-target',
          isToday
            ? 'bg-primary-container text-on-primary-container'
            : 'bg-surface-container-lowest text-on-surface hover:bg-surface-container-low/60',
          isSelected ? 'border-primary' : isToday ? 'border-transparent' : 'ghost-border',
          day?.beforeHistory === true && 'opacity-50',
        )}
      >
        <span className={clsx('text-xs', !isToday && 'text-on-surface-variant')}>
          {formatCivilDate(date, language, { weekday: 'short' })}
        </span>
        <span className="font-mono text-sm font-medium">{formatCivilDate(date, language, { day: 'numeric' })}</span>
        <span
          className={clsx(
            'flex w-full items-center justify-center gap-0.5 truncate text-center text-xs',
            !isToday && 'text-on-surface-variant',
          )}
        >
          {done && <Check aria-hidden="true" className="h-3 w-3 shrink-0 text-primary" />}
          <span className="truncate">{hint}</span>
        </span>
      </button>
    </li>
  );
}

/** The arrows either side of the week, and the way into the month. */
function Pager({
  canGoBack,
  canGoForward,
  onBack,
  onForward,
  onToday,
  onMonth,
  showToday,
}: {
  canGoBack: boolean;
  canGoForward: boolean;
  onBack: () => void;
  onForward: () => void;
  onToday: () => void;
  onMonth: () => void;
  showToday: boolean;
}) {
  const { t } = useTranslation();
  const arrow =
    'flex h-9 w-9 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring disabled:opacity-40 disabled:hover:bg-transparent';
  return (
    <>
      {showToday && (
        <button
          type="button"
          data-testid="home-week-today"
          onClick={onToday}
          className="rounded px-1 text-xs font-medium text-primary transition-colors hover:text-primary-hover focus-ring"
        >
          {t('plan.card.thisWeek')}
        </button>
      )}
      <button
        type="button"
        data-testid="home-week-previous"
        aria-label={t('home.calendar.previousWeek')}
        title={t('home.calendar.previousWeek')}
        disabled={!canGoBack}
        onClick={onBack}
        className={arrow}
      >
        <ChevronLeft aria-hidden="true" className="h-4 w-4" />
      </button>
      <button
        type="button"
        data-testid="home-week-next"
        aria-label={t('home.calendar.nextWeek')}
        title={t('home.calendar.nextWeek')}
        disabled={!canGoForward}
        onClick={onForward}
        className={arrow}
      >
        <ChevronRight aria-hidden="true" className="h-4 w-4" />
      </button>
      <button
        type="button"
        data-testid="home-week-month"
        aria-label={t('home.calendar.showMonth')}
        title={t('home.calendar.showMonth')}
        aria-haspopup="dialog"
        onClick={onMonth}
        className={arrow}
      >
        <CalendarDays aria-hidden="true" className="h-4 w-4" />
      </button>
    </>
  );
}

/**
 * The week section on the athlete's today, with or without a plan. The
 * strip opens on this week and pages a week at a time: back to the first
 * week the activity cache holds, forward to the plan's last week (next week
 * without a plan). A tap on a day opens it under the strip.
 */
function WeekStrip({
  plan,
  today,
  onOpenChatDraft,
  onNavigate,
}: {
  plan: WorkoutPlan | null;
  today: string;
  onOpenChatDraft: (text: string) => void;
  onNavigate: (route: string) => void;
}) {
  const { t, language } = useTranslation();
  const thisMonday = mondayOf(today);
  const [monday, setMonday] = useState(thisMonday);
  const [selected, setSelected] = useState<string | null>(null);
  const [monthOpen, setMonthOpen] = useState(false);
  const days = weekDays(monday);
  const calendar = useHomeCalendar(days[0], days[DAYS_PER_WEEK - 1]);
  // A placeholder is the previous week's answer: its days are not these.
  const answer = calendar.isPlaceholderData ? undefined : calendar.data;
  const historyStart = calendar.data?.history_start ?? null;
  const isThisWeek = monday === thisMonday;
  const thisWeekPlan = plan?.weeks.find((week) => week.week_start === monday);
  const shownWeek = answer?.plan_weeks?.find((week) => week.week_start === monday) ?? thisWeekPlan;
  const currentIndex = plan?.weeks.findIndex((week) => week.current) ?? -1;
  const nextIndex = plan?.weeks.findIndex((week) => week.week_start === addCivilDays(thisMonday, DAYS_PER_WEEK)) ?? -1;
  const cells = days.map((date) => ({ date, day: answer === undefined ? null : calendarDay(answer, date) }));
  const open = cells.find((cell) => cell.date === selected && cell.day !== null) ?? null;

  const page = (weeks: number) => {
    setMonday((current) => addCivilDays(current, DAYS_PER_WEEK * weeks));
    setSelected(null);
  };

  return (
    <Section
      title={
        isThisWeek
          ? t('plan.card.thisWeek')
          : t('plan.card.weekOf', { date: formatCivilDate(monday, language, { day: 'numeric', month: 'long' }) })
      }
      description={shownWeek?.focus ? shownWeek.focus : undefined}
      headingLevel={3}
      data-testid="home-week"
      actions={
        <Pager
          canGoBack={historyStart === null || monday > historyStart}
          canGoForward={monday < lastMonday(plan, today)}
          onBack={() => page(-1)}
          onForward={() => page(1)}
          onToday={() => {
            setMonday(thisMonday);
            setSelected(null);
          }}
          onMonth={() => setMonthOpen(true)}
          showToday={!isThisWeek}
        />
      }
    >
      {calendar.isError && answer === undefined ? (
        <EmptyState
          data-testid="home-week-failed"
          action={{ label: t('common.retry'), onClick: () => void calendar.refetch() }}
        >
          {t('home.calendar.loadFailed')}
        </EmptyState>
      ) : (
        <ol className="grid grid-cols-7 gap-1" aria-busy={answer === undefined}>
          {cells.map(({ date, day }) => (
            <DayCell
              key={date}
              date={date}
              day={day}
              today={today}
              isSelected={date === selected}
              onToggle={(clicked) => setSelected((current) => (current === clicked ? null : clicked))}
            />
          ))}
        </ol>
      )}
      {open?.day && (
        <CalendarDayDetail
          day={open.day}
          today={today}
          id={DETAIL_ID}
          onOpenChatDraft={onOpenChatDraft}
          onNavigate={onNavigate}
        />
      )}
      {answer !== undefined && historyStart !== null && monday <= historyStart && (
        <p data-testid="home-week-history-start" className="mt-3 text-sm text-on-surface-variant">
          {t('home.calendar.historyStart', { date: formatCivilDate(historyStart, language, DRAFT_DATE) })}
        </p>
      )}
      {answer !== undefined && answer.plan_weeks === null && (
        <p data-testid="home-week-no-plan" className="mt-3 text-sm text-on-surface-variant">
          {t('home.calendar.noPlan')}
        </p>
      )}
      {isThisWeek && plan !== null && nextIndex >= 0 && (
        <div data-testid="home-next-week" className="mt-4">
          <WeekHeading week={plan.weeks[nextIndex]} index={nextIndex} currentIndex={currentIndex} />
        </div>
      )}
      {monthOpen && (
        <HomeMonthSheet
          plan={plan}
          today={today}
          startMonth={monday}
          onClose={() => setMonthOpen(false)}
          onOpenChatDraft={(text) => {
            setMonthOpen(false);
            onOpenChatDraft(text);
          }}
          onNavigate={(route) => {
            setMonthOpen(false);
            onNavigate(route);
          }}
        />
      )}
    </Section>
  );
}

/**
 * Home's week section. It reads the athlete's today from the plan read the
 * Today section already holds — the same answer, never a second clock — and
 * draws nothing until that read has answered, which the Today section above
 * it shows as loading or failed. An athlete without a plan still gets the
 * strip, with what they did on each day.
 */
export function HomeWeek({ onOpenChatDraft, onNavigate }: HomeWeekProps) {
  const plan = useTrainingPlan();
  if (plan.data === undefined) return null;
  try {
    mondayOf(plan.data.today);
  } catch (error) {
    // A today that is not a calendar day is the Today section's load error.
    if (error instanceof RangeError) return null;
    throw error;
  }
  return (
    <WeekStrip
      key={plan.data.today}
      plan={plan.data.plan}
      today={plan.data.today}
      onOpenChatDraft={onOpenChatDraft}
      onNavigate={onNavigate}
    />
  );
}
