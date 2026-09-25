// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's week — a Monday-to-Sunday strip with today marked, a day that opens to its steps and fuel
// ABOUTME: The day detail and the next-week line are the plan card's own DayRow and WeekHeading, not a second rendering

import { useState } from 'react';
import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import type { PlanDayLookup, WorkoutPlan } from '@pierre/shared-types';
import { planDayOn } from '@pierre/shared-types';
import { Section } from '../ui/Section';
import { DayRow, WeekHeading } from '../chat/WorkoutPlanCard';
import { DRAFT_DATE, formatCivilDate, planDayDraft, sessionFacts, sportLabel, type PlanWindow } from './homeFormat';

interface HomeWeekProps {
  plan: WorkoutPlan;
  calendar: PlanWindow;
  onOpenChatDraft: (text: string) => void;
}

const DETAIL_ID = 'home-week-detail';

/** The few characters a strip cell has room for: the minutes, else the sport; rest is the word. */
function cellStatus(t: TFunction, lookup: PlanDayLookup): string {
  if (lookup.kind === 'rest') return t('chat.restDay');
  if (lookup.kind === 'uncovered') return '—';
  if (lookup.day.duration_min !== undefined) return `${lookup.day.duration_min} min`;
  return typeof lookup.day.sport === 'string' ? sportLabel(t, lookup.day.sport) : '';
}

/** What a screen reader hears for a cell, in full — the cell itself only has room for a hint. */
function cellLabel(t: TFunction, named: string, lookup: PlanDayLookup): string {
  if (lookup.kind === 'rest') return `${named} — ${t('chat.restDay')}`;
  if (lookup.kind === 'uncovered') return `${named} — ${t('home.plan.notCovered')}`;
  return `${named} — ${[lookup.day.workout, ...sessionFacts(t, lookup.day)].join(' · ')}`;
}

/** One day of the strip: weekday, date, and a hint of the session. */
function DayCell({
  date,
  lookup,
  isToday,
  isSelected,
  onToggle,
}: {
  date: string;
  lookup: PlanDayLookup;
  isToday: boolean;
  isSelected: boolean;
  onToggle: (date: string) => void;
}) {
  const { t, language } = useTranslation();
  return (
    <li className="min-w-0">
      <button
        type="button"
        data-testid={`home-week-day-${date}`}
        aria-expanded={isSelected}
        aria-controls={isSelected ? DETAIL_ID : undefined}
        aria-current={isToday ? 'date' : undefined}
        aria-label={cellLabel(t, formatCivilDate(date, language, DRAFT_DATE), lookup)}
        onClick={() => onToggle(date)}
        className={clsx(
          'flex w-full flex-col items-center gap-0.5 rounded-lg border px-0.5 py-1.5 transition-colors focus-ring touch-target',
          isToday
            ? 'bg-primary-container text-on-primary-container'
            : 'bg-surface-container-lowest text-on-surface hover:bg-surface-container-low/60',
          isSelected ? 'border-primary' : isToday ? 'border-transparent' : 'ghost-border',
        )}
      >
        <span className={clsx('text-xs', !isToday && 'text-on-surface-variant')}>
          {formatCivilDate(date, language, { weekday: 'short' })}
        </span>
        <span className="font-mono text-sm font-medium">{formatCivilDate(date, language, { day: 'numeric' })}</span>
        <span className={clsx('w-full truncate text-center text-xs', !isToday && 'text-on-surface-variant')}>
          {cellStatus(t, lookup)}
        </span>
      </button>
    </li>
  );
}

/**
 * The open day: the plan card's row for it — session, steps, template, fuel —
 * and the question it opens in chat, written out as the link itself.
 */
function DayDetail({
  date,
  lookup,
  onOpenChatDraft,
}: {
  date: string;
  lookup: PlanDayLookup;
  onOpenChatDraft: (text: string) => void;
}) {
  const { t, language } = useTranslation();
  const draft = planDayDraft(t, language, date, lookup);
  return (
    <div
      id={DETAIL_ID}
      data-testid={DETAIL_ID}
      className="mt-3 rounded-[10px] border ghost-border bg-surface-container-lowest px-3 py-2"
    >
      {lookup.kind === 'uncovered' || draft === null ? (
        <p className="py-1 text-sm text-on-surface-variant">{t('home.plan.notCovered')}</p>
      ) : (
        <>
          <table className="w-full text-sm">
            <tbody>
              <DayRow day={lookup.day} />
            </tbody>
          </table>
          <button
            type="button"
            onClick={() => onOpenChatDraft(draft)}
            className="mt-1 rounded py-1 text-left text-sm font-medium text-primary transition-colors hover:text-primary-hover focus-ring"
          >
            {draft}
          </button>
        </>
      )}
    </div>
  );
}

/** The week section: the strip, the open day under it, and next week's focus. */
export function HomeWeek({ plan, calendar, onOpenChatDraft }: HomeWeekProps) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<string | null>(null);
  const thisWeek = plan.weeks.find((week) => week.week_start === calendar.week[0]);
  const currentIndex = plan.weeks.findIndex((week) => week.current);
  const nextIndex = plan.weeks.findIndex((week) => week.week_start === calendar.nextMonday);
  const lookups = calendar.week.map((date) => ({ date, lookup: planDayOn(plan, date) }));
  const open = lookups.find((entry) => entry.date === selected) ?? null;

  return (
    <Section
      title={t('plan.card.thisWeek')}
      description={thisWeek?.focus ? thisWeek.focus : undefined}
      headingLevel={3}
      data-testid="home-week"
    >
      <ol className="grid grid-cols-7 gap-1">
        {lookups.map(({ date, lookup }) => (
          <DayCell
            key={date}
            date={date}
            lookup={lookup}
            isToday={date === calendar.today}
            isSelected={date === selected}
            onToggle={(day) => setSelected((current) => (current === day ? null : day))}
          />
        ))}
      </ol>
      {open !== null && <DayDetail date={open.date} lookup={open.lookup} onOpenChatDraft={onOpenChatDraft} />}
      {nextIndex >= 0 && (
        <div data-testid="home-next-week" className="mt-4">
          <WeekHeading week={plan.weeks[nextIndex]} index={nextIndex} currentIndex={currentIndex} />
        </div>
      )}
    </Section>
  );
}
