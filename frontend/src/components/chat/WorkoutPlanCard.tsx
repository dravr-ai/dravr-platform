// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the workout_plan block — the saved plan's season and fortnight — as the thread's one hairline card.
// ABOUTME: Reads the projection the platform built from the rows save_training_plan wrote; the agent's prose never feeds it.

import { useId, type ReactNode } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import { fuelParts, stepDuration } from '@pierre/chat-utils';
import { countKey, PLAN_HARD_PER_WEEK_KEYS, PLAN_MORE_WEEKS_KEYS } from '@pierre/shared-constants';
import {
  civilDayMonth,
  dayIsUncheckedEdit,
  formatCivilDate,
  ROW_DATE,
  SESSION_EDITOR_KEYS,
  sportLabel,
} from '@pierre/ui-logic';
import { BADGE_TONE, Badge } from '../ui';
import { EvidenceChip } from './EvidenceChip';
import type {
  PlanDay,
  PlanPhase,
  PlanStep,
  PlanWeek,
  WorkoutPlan,
} from '@pierre/shared-types';

/** Where one entry sits on the card, for the action drawn under it. */
export interface PlanEntryPlace {
  /** The id of the date's cell, on the date's first row, for the action to be described by. */
  dateId: string;
  /** The id of the entry's own title (or its rest word), telling a date's entries apart. */
  sessionId: string;
  /** Every entry on the date, in order, this one included. */
  entries: readonly PlanDay[];
  /** The entry's place among them, from 0. */
  position: number;
}

/**
 * What a card draws at the end of one entry, rest days included — the coach's
 * "Edit day" in their plan thread, and "Add a session" under a date's last.
 */
export type PlanDayAction = (day: PlanDay, place: PlanEntryPlace) => ReactNode;

interface WorkoutPlanCardProps {
  plan: WorkoutPlan;
  /** Drawn under the plan, inside the card — a coach's draft controls. */
  children?: ReactNode;
  /** Drawn at the end of every entry; absent on every card but a coach's in their plan thread. */
  dayAction?: PlanDayAction;
}

/**
 * A plan's calendar day with its year — "14 nov. 2026" — for the goal race,
 * the other races and the season's ends, spelled in the athlete's language
 * so it reads as a date rather than as the wire's ISO data.
 */
function planDate(date: string, language: string): string {
  return formatCivilDate(date, language, { day: 'numeric', month: 'short', year: 'numeric' });
}

/** "label · 4 min · zone", with "×repeat" appended when the step is repeated. */
function stepText(step: PlanStep): string {
  const text = [step.label, stepDuration(step.duration_seconds), step.target_zone].join(' · ');
  return step.repeat !== undefined && step.repeat > 1 ? `${text} ×${step.repeat}` : text;
}

/**
 * A session's steps, each as "label · 4 min · zone ×repeat", its note on hover.
 * One list for the plan card's day and the day editor's preview, so a
 * step reads the same in both. `className` lays the list out: inline on the
 * card's day, one step a line in the editor.
 */
export function PlanStepList({ steps, className }: { steps: readonly PlanStep[]; className?: string }) {
  return (
    <ol className={className}>
      {steps.map((step, index) => (
        <li key={`${step.label}-${index}`} title={step.note}>
          {stepText(step)}
        </li>
      ))}
    </ol>
  );
}

/**
 * The current phase's rhythm — its intent, then the weekly hours and hard
 * sessions the kernel set for it — as one line under the timeline.
 */
function phaseSummary(phase: PlanPhase, t: TFunction, language: string): string {
  const parts = [phase.intent || phase.purpose];
  if (phase.target_hours !== undefined) {
    parts.push(t('plan.card.hoursPerWeek', { hours: phase.target_hours }));
  }
  if (phase.hard_sessions_max !== undefined) {
    parts.push(t(countKey(phase.hard_sessions_max, language, PLAN_HARD_PER_WEEK_KEYS), { count: phase.hard_sessions_max }));
  }
  return parts.filter((part) => part.length > 0).join(' · ');
}

/**
 * One segment of the season timeline: the phase's kind and length, and the
 * `now` badge on the phase covering the athlete's today. The kind label
 * defaults to the kind itself so a kind the catalogue has not named still
 * reads as a word rather than as a key.
 */
function PhaseSegment({ phase }: { phase: PlanPhase }) {
  const { t } = useTranslation();
  return (
    <li
      className="flex items-baseline gap-1.5 rounded-md bg-surface-container-high px-2 py-1 text-xs text-on-surface"
      title={phase.purpose}
    >
      <span className="font-medium">{t(`plan.card.phase.${phase.kind}`, { defaultValue: phase.kind })}</span>
      <span className="text-on-surface-variant">{t('plan.card.weeksShort', { count: phase.weeks })}</span>
      {phase.current && (
        <span className={`rounded-full px-1.5 font-medium ${BADGE_TONE.accent.tone}`}>
          {t('plan.card.now')}
        </span>
      )}
    </li>
  );
}

/**
 * The season: the flavour with who chose it, then the phases in order as a
 * compact row, then the current phase's rhythm.
 */
function Season({ plan }: { plan: WorkoutPlan }) {
  const { t, language } = useTranslation();
  const { flavour, phases } = plan;
  const current = phases.find((phase) => phase.current);
  if (!flavour && phases.length === 0) {
    return null;
  }
  return (
    <section className="border-b ghost-border-faint px-3 py-2.5">
      {flavour && (
        <div className="mb-2 text-sm">
          <span className="text-on-surface-variant">{t('plan.card.approach')}</span>{' '}
          <span className="font-medium text-on-surface">{flavour.label}</span>
          <span className="text-xs text-on-surface-variant">
            {' · '}
            {t(`plan.card.selectedBy.${flavour.selected_by}`, { defaultValue: flavour.selected_by })}
          </span>
        </div>
      )}
      {phases.length > 0 && (
        <>
          <div className="mb-1.5 flex flex-wrap items-baseline gap-x-2 text-xs text-on-surface-variant">
            <span>{t('plan.card.season')}</span>
            {plan.season_start && plan.season_end && (
              <span className="font-mono">
                {planDate(plan.season_start, language)} → {planDate(plan.season_end, language)}
              </span>
            )}
          </div>
          <ol className="flex flex-wrap gap-1">
            {phases.map((phase) => (
              <PhaseSegment key={`${phase.kind}-${phase.start}`} phase={phase} />
            ))}
          </ol>
          {current && (
            <p className="mt-1.5 text-xs text-on-surface-variant">{phaseSummary(current, t, language)}</p>
          )}
        </>
      )}
    </section>
  );
}

/**
 * A week's entries by date, in the order the dates come, each date's entries
 * in the order they are done: a date may hold several (carnet#1716).
 */
function entriesByDate(days: readonly PlanDay[]): PlanDay[][] {
  const byDate = new Map<string, PlanDay[]>();
  for (const day of days) {
    const entries = byDate.get(day.date);
    if (entries) entries.push(day);
    else byDate.set(day.date, [day]);
  }
  return [...byDate.values()];
}

/**
 * One date of the fortnight: a row per entry, in order, the date's cell on
 * its first row spanning the others. A row's keys and test ids carry its
 * place among the date's entries, which is its position (carnet#1716).
 *
 * The card lays a week out as a table, and the Home page's day detail draws
 * the same rows in a table of their own, so a date reads the same in both
 * places. `action` is drawn last in each entry's session cell.
 */
export function PlanDateRows({ entries, action }: { entries: readonly PlanDay[]; action?: PlanDayAction }) {
  const dateId = useId();
  return (
    <>
      {entries.map((day, position) => (
        <DayRow
          key={`${day.date}-${position}`}
          day={day}
          place={{ dateId, entries, position }}
          action={action}
        />
      ))}
    </>
  );
}

/**
 * One entry of a date: the date on the date's first row, the sport, and the
 * session — its text, duration and intensity, then the steps, the template
 * and the fuel it carries. A rest entry is the one word. An entry the coach
 * edited in their draft carries "Edited", one they added "Added", and a
 * session they wrote by hand says it was not checked against a family's
 * evidence (carnet#1714, #1716).
 */
function DayRow({
  day,
  place,
  action,
}: {
  day: PlanDay;
  place: Omit<PlanEntryPlace, 'sessionId'>;
  action?: PlanDayAction;
}) {
  const { t, language } = useTranslation();
  const sessionId = useId();
  const { dateId, entries, position } = place;
  const meta = [day.duration_min !== undefined ? `${day.duration_min} min` : null, day.intensity].filter(
    (part): part is string => typeof part === 'string' && part.length > 0,
  );
  const steps = day.steps ?? [];
  // An added session is the coach's own from the start: "Added" says more than "Edited".
  const mark = day.added ? (
    <Badge variant="neutral" className="ml-2" data-testid="plan-day-added">
      {t(SESSION_EDITOR_KEYS.added)}
    </Badge>
  ) : (
    day.edited && (
      <Badge variant="neutral" className="ml-2" data-testid="plan-day-edited">
        {t(SESSION_EDITOR_KEYS.edited)}
      </Badge>
    )
  );
  return (
    <tr className="border-b ghost-border-faint align-top last:border-0" data-testid={`plan-day-${day.date}-${position}`}>
      {/* The date once, on its first entry, spanning the rest: the later
          entries' actions are described by this cell. */}
      {position === 0 && (
        <td
          id={action ? dateId : undefined}
          rowSpan={entries.length > 1 ? entries.length : undefined}
          className="whitespace-nowrap py-1.5 pr-3 font-mono text-xs text-on-surface-variant"
        >
          {formatCivilDate(day.date, language, ROW_DATE)}
        </td>
      )}
      {/* The sport in the athlete's words; a rest day has none, its one word
          is in the next cell. */}
      <td className="whitespace-nowrap py-1.5 pr-3 text-xs text-on-surface-variant">
        {day.rest ? null : sportLabel(t, day.sport)}
      </td>
      <td className="w-full py-1.5">
        {day.rest ? (
          <>
            <span id={sessionId} className="text-on-surface-variant">
              {t('chat.restDay')}
            </span>
            {mark}
          </>
        ) : (
          <>
            <span id={sessionId} className="font-medium text-on-surface">
              {day.workout}
            </span>
            {meta.length > 0 && (
              <span className="text-on-surface-variant">
                {' · '}
                {meta.join(' · ')}
              </span>
            )}
            {mark}
            {day.outside_evidence && (
              <Badge variant="warning" className="ml-2">
                {day.outside_evidence}
              </Badge>
            )}
            {day.evidence && <EvidenceChip evidence={day.evidence} />}
            {day.counts_as && (
              <div className="mt-0.5 text-xs text-on-surface-variant" data-testid="plan-day-counts-as">
                {day.counts_as}
              </div>
            )}
            {dayIsUncheckedEdit(day) && (
              <div className="mt-0.5 text-xs text-on-surface-variant" data-testid="plan-day-unchecked">
                {t(SESSION_EDITOR_KEYS.unchecked)}
              </div>
            )}
          </>
        )}
        {steps.length > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-on-surface-variant">
            <span className="font-medium text-on-surface">{t('plan.card.steps')}</span>
            <PlanStepList steps={steps} className="contents" />
          </div>
        )}
        {day.template_slug && (
          <div className="mt-0.5 text-xs text-on-surface-variant">
            {t('plan.card.template')} <span className="font-mono text-on-surface">{day.template_slug}</span>
            {day.template_source && (
              <>
                {' · '}
                {t(`plan.card.source.${day.template_source}`, { defaultValue: day.template_source })}
              </>
            )}
          </div>
        )}
        {day.fueling && (
          <div className="mt-0.5 text-xs text-on-surface-variant">
            <span className="font-medium text-on-surface">{t('chat.fuelLabel')}</span>{' '}
            {fuelParts(day.fueling, t).join(' · ')}
          </div>
        )}
        {day.fueling_withheld && (
          <div className="mt-0.5 text-xs text-on-surface-variant">
            <span className="font-medium text-on-surface">{t('chat.fuelLabel')}</span>{' '}
            {t('chat.fuelWithheld')}
          </div>
        )}
        {action && <div className="mt-1">{action(day, { dateId, sessionId, entries, position })}</div>}
      </td>
    </tr>
  );
}

/**
 * The week's heading: "This week" for the week covering today, "Next week"
 * for the one after it, the Monday's date for any other. The date rides in
 * mono beside the two named weeks so every heading still carries it. The Home
 * page's next-week line is this heading.
 */
export function WeekHeading({ week, index, currentIndex }: { week: PlanWeek; index: number; currentIndex: number }) {
  const { t, language } = useTranslation();
  const monday = civilDayMonth(week.week_start, language);
  const named = week.current
    ? t('plan.card.thisWeek')
    : currentIndex >= 0 && index === currentIndex + 1
      ? t('plan.card.nextWeek')
      : null;
  return (
    <div className="mb-1 flex flex-wrap items-baseline gap-x-2">
      <span className="text-xs font-semibold text-on-surface-variant">
        {named ?? t('plan.card.weekOf', { date: monday })}
      </span>
      {named && <span className="font-mono text-xs text-outline">{monday}</span>}
      {week.focus && <span className="text-xs text-on-surface-variant">{week.focus}</span>}
    </div>
  );
}

export default function WorkoutPlanCard({ plan, children, dayAction }: WorkoutPlanCardProps) {
  const { t, language } = useTranslation();
  const currentIndex = plan.weeks.findIndex((week) => week.current);

  return (
    <div className="my-2 overflow-hidden rounded-[10px] border ghost-border bg-surface-container-lowest">
      {/* Header — the plan is a data object inside the agent's turn, the one
          place a hairline card survives in the thread (DESIGN.md "Chat surfaces"). */}
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 border-b ghost-border-faint px-3 py-2.5">
        <span className="text-sm font-semibold text-on-surface">{t('chat.trainingPlanTitle')}</span>
        <span className="text-xs text-on-surface-variant">
          {t('plan.card.goalRace')}{' '}
          <span className="font-medium text-on-surface">{plan.goal_race.name}</span>
          {' · '}
          <span className="font-mono">{planDate(plan.goal_race.date, language)}</span>
        </span>
      </div>

      {plan.races !== undefined && plan.races.length > 0 && (
        <div className="border-b ghost-border-faint px-3 py-2 text-xs text-on-surface-variant">
          <span className="font-medium text-on-surface">{t('plan.card.alsoRacing')}</span>{' '}
          {plan.races.map((race, index) => (
            <span key={`${race.name}-${race.date}`}>
              {index > 0 && ' · '}
              {race.name} <span className="font-mono">{planDate(race.date, language)}</span> ({race.priority})
            </span>
          ))}
        </div>
      )}

      <Season plan={plan} />

      {plan.weeks.length > 0 && (
        <div className="px-3 py-2.5">
          {plan.weeks.map((week, index) => (
            <div key={week.week_start} className="mb-3 last:mb-0">
              <WeekHeading week={week} index={index} currentIndex={currentIndex} />
              <table className="w-full text-sm">
                <tbody>
                  {entriesByDate(week.days).map((entries) => (
                    <PlanDateRows key={entries[0]?.date} entries={entries} action={dayAction} />
                  ))}
                </tbody>
              </table>
            </div>
          ))}
        </div>
      )}

      {plan.weeks_deferred > 0 && (
        <div className="border-t ghost-border-faint px-3 py-2 text-xs text-on-surface-variant">
          {t(countKey(plan.weeks_deferred, language, PLAN_MORE_WEEKS_KEYS), { count: plan.weeks_deferred })}
        </div>
      )}

      {children}
    </div>
  );
}
