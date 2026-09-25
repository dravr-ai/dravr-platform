// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's Today section — the plan's session for the athlete's today, enlarged, and tomorrow's under it
// ABOUTME: A rest day says rest and a day the plan never reached says so; tapping a day drafts the question in chat

import { useTranslation } from '@pierre/i18n';
import type { PlanDayLookup, WorkoutPlan } from '@pierre/shared-types';
import { planDayOn } from '@pierre/shared-types';
import { Section } from '../ui/Section';
import { DRAFT_DATE, formatCivilDate, phaseWeekLabel, planDayDraft, sessionFacts, type PlanWindow } from './homeFormat';

interface HomeTodayProps {
  plan: WorkoutPlan;
  calendar: PlanWindow;
  onOpenChatDraft: (text: string) => void;
}

/**
 * Today's session as the page's one data card: the workout in the display
 * face, the phase and its week on the right, the sport, minutes and intensity
 * under it. The whole card is the way into the conversation about it.
 */
function TodayCard({
  lookup,
  phaseLabel,
  draft,
  onOpenChatDraft,
}: {
  lookup: PlanDayLookup;
  phaseLabel: string | null;
  draft: string | null;
  onOpenChatDraft: (text: string) => void;
}) {
  const { t } = useTranslation();
  if (lookup.kind === 'uncovered' || draft === null) {
    return (
      <p data-testid="home-today-uncovered" className="py-3 text-sm text-on-surface-variant">
        {t('home.plan.notCovered')}
      </p>
    );
  }
  const headline = lookup.kind === 'session' ? lookup.day.workout : t('chat.restDay');
  const detail =
    lookup.kind === 'session' ? sessionFacts(t, lookup.day).join(' · ') : lookup.week.focus;
  return (
    <button
      type="button"
      data-testid={lookup.kind === 'session' ? 'home-today-session' : 'home-today-rest'}
      onClick={() => onOpenChatDraft(draft)}
      className="card block w-full text-left transition-colors hover:bg-surface-container-low/60 focus-ring"
    >
      <span className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <span className="font-display text-lg font-semibold text-on-surface">{headline}</span>
        {phaseLabel !== null && <span className="text-xs text-on-surface-variant">{phaseLabel}</span>}
      </span>
      {detail && <span className="mt-1 block text-sm text-on-surface-variant">{detail}</span>}
    </button>
  );
}

/** "Tomorrow" and what the plan holds for it, on one line. */
function TomorrowLine({
  lookup,
  draft,
  onOpenChatDraft,
}: {
  lookup: PlanDayLookup;
  draft: string | null;
  onOpenChatDraft: (text: string) => void;
}) {
  const { t } = useTranslation();
  const value =
    lookup.kind === 'session'
      ? [lookup.day.workout, lookup.day.duration_min !== undefined ? `${lookup.day.duration_min} min` : null]
          .filter((part): part is string => typeof part === 'string' && part.length > 0)
          .join(' · ')
      : lookup.kind === 'rest'
        ? t('chat.restDay')
        : t('home.plan.notCovered');
  return (
    <div data-testid="home-tomorrow" className="mt-3 flex min-w-0 items-baseline gap-3 text-sm">
      <span className="shrink-0 text-on-surface-variant">{t('home.plan.tomorrow')}</span>
      {draft === null ? (
        <span className="min-w-0 text-on-surface-variant">{value}</span>
      ) : (
        <button
          type="button"
          onClick={() => onOpenChatDraft(draft)}
          className="min-w-0 truncate rounded text-left font-medium text-on-surface transition-colors hover:text-primary focus-ring"
        >
          {value}
        </button>
      )}
    </div>
  );
}

/** The Today section: the athlete's today by the plan, then tomorrow. */
export function HomeToday({ plan, calendar, onOpenChatDraft }: HomeTodayProps) {
  const { t, language } = useTranslation();
  const today = planDayOn(plan, calendar.today);
  const tomorrow = planDayOn(plan, calendar.tomorrow);
  return (
    <Section
      title={t('chat.dayToday')}
      headingLevel={3}
      data-testid="home-today"
      actions={
        <span className="font-mono text-xs text-on-surface-variant">
          {formatCivilDate(calendar.today, language, DRAFT_DATE)}
        </span>
      }
    >
      <TodayCard
        lookup={today}
        phaseLabel={today.kind === 'uncovered' ? null : phaseWeekLabel(t, plan.phases, calendar.today)}
        draft={planDayDraft(t, language, calendar.today, today)}
        onOpenChatDraft={onOpenChatDraft}
      />
      <TomorrowLine
        lookup={tomorrow}
        draft={planDayDraft(t, language, calendar.tomorrow, tomorrow)}
        onOpenChatDraft={onOpenChatDraft}
      />
    </Section>
  );
}
