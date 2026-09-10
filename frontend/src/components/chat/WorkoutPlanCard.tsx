// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the workout_plan block — the saved plan's season and fortnight — as the thread's one hairline card.
// ABOUTME: Reads the projection the platform built from the rows save_training_plan wrote; the agent's prose never feeds it.

import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import type {
  PlanDay,
  PlanFueling,
  PlanPhase,
  PlanStep,
  PlanWeek,
  WorkoutPlan,
} from '@pierre/shared-types';

interface WorkoutPlanCardProps {
  plan: WorkoutPlan;
}

/**
 * A step's duration as the compact figure the steps line carries: whole
 * minutes once a step lasts one, seconds under that so a stride does not
 * read as "0m".
 */
function stepDuration(seconds: number): string {
  return seconds < 60 ? `${seconds}s` : `${Math.round(seconds / 60)}m`;
}

/** "label · Nm · zone", with "×repeat" appended when the step is repeated. */
function stepText(step: PlanStep): string {
  const text = [step.label, stepDuration(step.duration_seconds), step.target_zone].join(' · ');
  return step.repeat !== undefined && step.repeat > 1 ? `${text} ×${step.repeat}` : text;
}

/**
 * The fuelling fragments for one day, in display order. Sodium is shown as
 * an estimated sweat loss and never as a required intake: the evidence does
 * not support prescribing a mg/h figure.
 */
function fuelParts(fueling: PlanFueling, t: TFunction): string[] {
  const parts = [
    t('chat.fuelCarbs', { value: fueling.carbs_g_per_h }),
    t('chat.fuelFluid', { value: fueling.fluid_ml_per_h }),
  ];
  if (fueling.sodium_mg_per_h !== undefined) {
    parts.push(t('chat.fuelSodiumLoss', { value: fueling.sodium_mg_per_h }));
  }
  return parts;
}

/**
 * The current phase's rhythm — its intent, then the weekly hours and hard
 * sessions the kernel set for it — as one line under the timeline.
 */
function phaseSummary(phase: PlanPhase, t: TFunction): string {
  const parts = [phase.intent || phase.purpose];
  if (phase.target_hours !== undefined) {
    parts.push(t('plan.card.hoursPerWeek', { hours: phase.target_hours }));
  }
  if (phase.hard_sessions_max !== undefined) {
    parts.push(t('plan.card.hardPerWeek', { count: phase.hard_sessions_max }));
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
        <span className="rounded-full bg-primary-container px-1.5 font-medium text-on-primary-container">
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
  const { t } = useTranslation();
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
                {plan.season_start} → {plan.season_end}
              </span>
            )}
          </div>
          <ol className="flex flex-wrap gap-1">
            {phases.map((phase) => (
              <PhaseSegment key={`${phase.kind}-${phase.start}`} phase={phase} />
            ))}
          </ol>
          {current && (
            <p className="mt-1.5 text-xs text-on-surface-variant">{phaseSummary(current, t)}</p>
          )}
        </>
      )}
    </section>
  );
}

/**
 * One day of the fortnight: the date, the sport, and the session — its text,
 * duration and intensity, then the steps, the template and the fuel it
 * carries. A rest day is the one word.
 */
function DayRow({ day }: { day: PlanDay }) {
  const { t } = useTranslation();
  const meta = [day.duration_min !== undefined ? `${day.duration_min} min` : null, day.intensity].filter(
    (part): part is string => typeof part === 'string' && part.length > 0,
  );
  const steps = day.steps ?? [];
  return (
    <tr className="border-b ghost-border-faint align-top last:border-0">
      <td className="whitespace-nowrap py-1.5 pr-3 font-mono text-xs text-on-surface-variant">{day.date}</td>
      <td className="whitespace-nowrap py-1.5 pr-3 text-xs text-on-surface-variant">{day.sport}</td>
      <td className="w-full py-1.5">
        {day.rest ? (
          <span className="text-on-surface-variant">{t('chat.restDay')}</span>
        ) : (
          <>
            <span className="font-medium text-on-surface">{day.workout}</span>
            {meta.length > 0 && (
              <span className="text-on-surface-variant">
                {' · '}
                {meta.join(' · ')}
              </span>
            )}
          </>
        )}
        {steps.length > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-on-surface-variant">
            <span className="font-medium text-on-surface">{t('plan.card.steps')}</span>
            <ol className="contents">
              {steps.map((step, index) => (
                <li key={`${step.label}-${index}`} title={step.note}>
                  {stepText(step)}
                </li>
              ))}
            </ol>
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
      </td>
    </tr>
  );
}

/**
 * The week's heading: "This week" for the week covering today, "Next week"
 * for the one after it, the Monday's date for any other. The date rides in
 * mono beside the two named weeks so every heading still carries it.
 */
function WeekHeading({ week, index, currentIndex }: { week: PlanWeek; index: number; currentIndex: number }) {
  const { t } = useTranslation();
  const named = week.current
    ? t('plan.card.thisWeek')
    : currentIndex >= 0 && index === currentIndex + 1
      ? t('plan.card.nextWeek')
      : null;
  return (
    <div className="mb-1 flex flex-wrap items-baseline gap-x-2">
      <span className="text-xs font-semibold text-on-surface-variant">
        {named ?? t('plan.card.weekOf', { date: week.week_start })}
      </span>
      {named && <span className="font-mono text-xs text-outline">{week.week_start}</span>}
      {week.focus && <span className="text-xs text-on-surface-variant">{week.focus}</span>}
    </div>
  );
}

export default function WorkoutPlanCard({ plan }: WorkoutPlanCardProps) {
  const { t } = useTranslation();
  const currentIndex = plan.weeks.findIndex((week) => week.current);

  return (
    <div className="my-2 overflow-hidden rounded-[10px] border ghost-border bg-surface-container-lowest">
      {/* Header — the plan is a data object inside the agent's turn, the one
          place a hairline card survives in the thread (DESIGN.md §5). */}
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 border-b ghost-border-faint px-3 py-2.5">
        <span className="text-sm font-semibold text-on-surface">{t('chat.trainingPlanTitle')}</span>
        <span className="text-xs text-on-surface-variant">
          {t('plan.card.goalRace')}{' '}
          <span className="font-medium text-on-surface">{plan.goal_race.name}</span>
          {' · '}
          <span className="font-mono">{plan.goal_race.date}</span>
        </span>
      </div>

      {plan.races !== undefined && plan.races.length > 0 && (
        <div className="border-b ghost-border-faint px-3 py-2 text-xs text-on-surface-variant">
          <span className="font-medium text-on-surface">{t('plan.card.alsoRacing')}</span>{' '}
          {plan.races.map((race, index) => (
            <span key={`${race.name}-${race.date}`}>
              {index > 0 && ' · '}
              {race.name} <span className="font-mono">{race.date}</span> ({race.priority})
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
                  {week.days.map((day) => (
                    <DayRow key={day.date} day={day} />
                  ))}
                </tbody>
              </table>
            </div>
          ))}
        </div>
      )}

      {plan.weeks_deferred > 0 && (
        <div className="border-t ghost-border-faint px-3 py-2 text-xs text-on-surface-variant">
          {t('plan.card.moreWeeks', { count: plan.weeks_deferred })}
        </div>
      )}
    </div>
  );
}
