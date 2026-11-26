// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's Today section — the plan's sessions for the athlete's today, enlarged, and tomorrow's under them
// ABOUTME: A rest day says rest and a day the plan never reached says so; tapping an entry drafts the question about it in chat

import { MapPin } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import type { PlanDay, PlanDayLookup, PlanWeek, WorkoutPlan } from '@pierre/shared-types';
import { planDayOn } from '@pierre/shared-types';
import {
  DRAFT_DATE,
  formatCivilDate,
  phaseWeekLabel,
  planDayDraft,
  planDayRouteDraft,
  type PlanWindow,
  sessionFacts,
  useDistanceUnit,
} from '@pierre/ui-logic';
import { Section } from '../ui/Section';

interface HomeTodayProps {
  plan: WorkoutPlan;
  calendar: PlanWindow;
  onOpenChatDraft: (text: string) => void;
}

/**
 * One of today's entries as a data card: the workout in the display face,
 * the sport, minutes and intensity under it; a rest entry says rest over the
 * week's focus. The phase and its week ride on the first card's right. The
 * whole card is the way into the conversation about that entry, and a test
 * id carries its position among the date's entries (carnet#1716).
 */
function TodayCard({
  day,
  position,
  week,
  phaseLabel,
  onOpenChatDraft,
}: {
  day: PlanDay;
  position: number;
  week: PlanWeek;
  phaseLabel: string | null;
  onOpenChatDraft: (text: string) => void;
}) {
  const { t, language } = useTranslation();
  const headline = day.rest ? t('chat.restDay') : day.workout;
  const detail = day.rest ? week.focus : sessionFacts(t, day).join(' · ');
  return (
    <button
      type="button"
      data-testid={`${day.rest ? 'home-today-rest' : 'home-today-session'}-${position}`}
      onClick={() => onOpenChatDraft(planDayDraft(t, language, day))}
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

/**
 * The way from one of today's sessions to a route for it: an ink link under
 * its card that drafts the question in chat, where the agent searches real
 * routes near the place the athlete names.
 */
function RouteLink({
  draft,
  position,
  onOpenChatDraft,
}: {
  draft: string;
  position: number;
  onOpenChatDraft: (text: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      data-testid={`home-today-route-${position}`}
      onClick={() => onOpenChatDraft(draft)}
      className="mt-2 inline-flex items-center gap-1.5 rounded text-sm font-medium text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
    >
      <MapPin aria-hidden="true" className="h-4 w-4" />
      <span>{t('home.plan.routeCta')}</span>
    </button>
  );
}

/** One entry in a few words: its workout and minutes, or rest. */
function entryWords(t: TFunction, day: PlanDay): string {
  if (day.rest) return t('chat.restDay');
  return [day.workout, day.duration_min !== undefined ? `${day.duration_min} min` : null]
    .filter((part): part is string => typeof part === 'string' && part.length > 0)
    .join(' · ');
}

/**
 * Today with nothing in the plan, said once and with a way forward: the line
 * names tomorrow too when it is empty as well, so the section never prints
 * the same sentence twice, and its link drafts the request to fill the week
 * (carnet#1840).
 */
function UncoveredLine({ alsoTomorrow, onOpenChatDraft }: { alsoTomorrow: boolean; onOpenChatDraft: (text: string) => void }) {
  const { t } = useTranslation();
  return (
    <div data-testid="home-today-uncovered" className="flex flex-wrap items-baseline gap-x-3 gap-y-1 py-3 text-sm">
      <span className="text-on-surface-variant">
        {t(alsoTomorrow ? 'home.plan.notCoveredBoth' : 'home.plan.notCoveredToday')}
      </span>
      <button
        type="button"
        data-testid="home-today-fill-week"
        onClick={() => onOpenChatDraft(t('home.plan.fillWeekDraft'))}
        className="rounded font-medium text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
      >
        {t('home.plan.fillWeekCta')}
      </button>
    </div>
  );
}

/** "Tomorrow" and what the plan holds for it: one line per entry, each drafting its own question. */
function TomorrowLine({ lookup, onOpenChatDraft }: { lookup: PlanDayLookup; onOpenChatDraft: (text: string) => void }) {
  const { t, language } = useTranslation();
  return (
    <div data-testid="home-tomorrow" className="mt-3 flex min-w-0 items-baseline gap-3 text-sm">
      <span className="shrink-0 text-on-surface-variant">{t('home.plan.tomorrow')}</span>
      {lookup.kind === 'uncovered' ? (
        <span className="min-w-0 text-on-surface-variant">{t('home.plan.nothingPlanned')}</span>
      ) : (
        <div className="min-w-0 space-y-1">
          {lookup.days.map((day, position) => (
            <button
              key={`${day.date}-${position}`}
              type="button"
              data-testid={`home-tomorrow-${position}`}
              onClick={() => onOpenChatDraft(planDayDraft(t, language, day))}
              className="block max-w-full truncate rounded text-left font-medium text-on-surface transition-colors hover:text-primary focus-ring"
            >
              {entryWords(t, day)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** The Today section: each of the athlete's entries today by the plan, in order, then tomorrow. */
export function HomeToday({ plan, calendar, onOpenChatDraft }: HomeTodayProps) {
  const { t, language } = useTranslation();
  const today = planDayOn(plan, calendar.today);
  const tomorrow = planDayOn(plan, calendar.tomorrow);
  const unit = useDistanceUnit();
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
      {today.kind === 'uncovered' ? (
        <UncoveredLine alsoTomorrow={tomorrow.kind === 'uncovered'} onOpenChatDraft={onOpenChatDraft} />
      ) : (
        <div className="space-y-2">
          {today.days.map((day, position) => {
            const routeDraft = planDayRouteDraft(t, language, day, unit);
            return (
              <div key={`${day.date}-${position}`}>
                <TodayCard
                  day={day}
                  position={position}
                  week={today.week}
                  phaseLabel={position === 0 ? phaseWeekLabel(t, plan.phases, calendar.today) : null}
                  onOpenChatDraft={onOpenChatDraft}
                />
                {routeDraft !== null && (
                  <RouteLink draft={routeDraft} position={position} onOpenChatDraft={onOpenChatDraft} />
                )}
              </div>
            );
          })}
        </div>
      )}
      {/* An empty tomorrow is already named by today's empty line. */}
      {!(today.kind === 'uncovered' && tomorrow.kind === 'uncovered') && (
        <TomorrowLine lookup={tomorrow} onOpenChatDraft={onOpenChatDraft} />
      )}
    </Section>
  );
}
