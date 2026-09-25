// ABOUTME: React Native card for the workout_plan block — the saved plan's season and fortnight, mirroring the web card.
// ABOUTME: Reads the projection the platform built from the rows save_training_plan wrote; the agent's prose never feeds it.

import React from 'react';
import { View, Text } from 'react-native';
import { fuelParts, stepDuration } from '@pierre/chat-utils';
import type {
  PlanDay,
  PlanPhase,
  PlanStep,
  PlanWeek,
  WorkoutPlan,
} from '@pierre/shared-types';

import { useTranslation } from '@pierre/i18n';

interface WorkoutPlanCardProps {
  plan: WorkoutPlan;
}

/** A figure inside a sentence: dates, durations, counts, set in the mono face so columns of them line up. */
const FIGURE = 'font-mono tabular-nums';

/**
 * A figure inside a sentence — a date, a duration, a count — set in the mono
 * face so two of them under each other line up.
 */
function Figure({ children }: { children: React.ReactNode }) {
  return <Text className={FIGURE}>{children}</Text>;
}

/**
 * A translated sentence with the figure it interpolates set in mono: the
 * sentence is split at the figure's first occurrence, so every locale keeps
 * its own word order. A sentence that does not carry the figure verbatim
 * renders as it is.
 */
function Sentence({ text, figure }: { text: string; figure: string | number }) {
  const needle = String(figure);
  const at = text.indexOf(needle);
  if (at < 0) {
    return <Text>{text}</Text>;
  }
  return (
    <Text>
      {text.slice(0, at)}
      <Figure>{needle}</Figure>
      {text.slice(at + needle.length)}
    </Text>
  );
}

/**
 * "label · Nm · zone", with "×repeat" appended when the step is repeated.
 * The duration and the repeat count are the step's two figures.
 */
function StepText({ step }: { step: PlanStep }) {
  return (
    <Text>
      {step.label}
      {' · '}
      <Figure>{stepDuration(step.duration_seconds)}</Figure>
      {' · '}
      {step.target_zone}
      {step.repeat !== undefined && step.repeat > 1 ? (
        <Text>
          {' '}
          <Figure>×{step.repeat}</Figure>
        </Text>
      ) : null}
    </Text>
  );
}

/**
 * The current phase's rhythm — its intent, then the weekly hours and hard
 * sessions the kernel set for it — as one line under the timeline. The two
 * weekly figures are mono; the intent is prose.
 */
function PhaseSummary({ phase }: { phase: PlanPhase }) {
  const { t } = useTranslation();
  const intent = phase.intent || phase.purpose;
  return (
    <Text className="text-sm text-text-secondary mt-2">
      {intent}
      {phase.target_hours !== undefined ? (
        <Text>
          {intent.length > 0 ? ' · ' : ''}
          <Figure>{t('plan.card.hoursPerWeek', { hours: phase.target_hours })}</Figure>
        </Text>
      ) : null}
      {phase.hard_sessions_max !== undefined ? (
        <Text>
          {' · '}
          <Figure>{t('plan.card.hardPerWeek', { count: phase.hard_sessions_max })}</Figure>
        </Text>
      ) : null}
    </Text>
  );
}

/**
 * One segment of the season timeline: the phase's kind and length, and the
 * word `now` on the phase covering the athlete's today, in the primary ink
 * like the phase it marks. The kind label defaults to the kind itself so a
 * kind the catalogue has not named still reads as a word rather than as a key.
 */
function PhaseSegment({ phase }: { phase: PlanPhase }) {
  const { t } = useTranslation();
  const ink = phase.current ? 'text-primary' : 'text-text-secondary';
  return (
    <View className="flex-row items-center mr-3 mb-1">
      <Text className={`text-sm font-medium ${ink}`}>
        {t(`plan.card.phase.${phase.kind}`, { defaultValue: phase.kind })}
      </Text>
      <Text className={`text-sm ${ink}`}>
        {' '}
        <Figure>{t('plan.card.weeksShort', { count: phase.weeks })}</Figure>
      </Text>
      {phase.current ? <Text className="text-sm text-primary ml-1">{t('plan.card.now')}</Text> : null}
    </View>
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
    <View className="px-4 mt-3">
      {flavour ? (
        <Text className="text-sm mb-2">
          <Text className="text-text-secondary">{t('plan.card.approach')} </Text>
          <Text className="font-semibold text-text-primary">{flavour.label}</Text>
          <Text className="text-text-secondary">
            {' · '}
            {t(`plan.card.selectedBy.${flavour.selected_by}`, { defaultValue: flavour.selected_by })}
          </Text>
        </Text>
      ) : null}
      {phases.length > 0 ? (
        <>
          <Text className="text-sm text-text-secondary mb-1">
            {t('plan.card.season')}
            {plan.season_start && plan.season_end ? (
              <Text>
                {'  '}
                <Figure>{plan.season_start}</Figure>
                {' → '}
                <Figure>{plan.season_end}</Figure>
              </Text>
            ) : null}
          </Text>
          <View className="flex-row flex-wrap">
            {phases.map((phase) => (
              <PhaseSegment key={`${phase.kind}-${phase.start}`} phase={phase} />
            ))}
          </View>
          {current ? <PhaseSummary phase={current} /> : null}
        </>
      ) : null}
    </View>
  );
}

/**
 * One day of the fortnight: the date, the sport, and the session — its text,
 * duration and intensity, then the steps, the template and the fuel it
 * carries. A rest day is the one word. Rows are separated by space, not by a
 * rule; the card's one hairline sits under its header.
 */
function DayRow({ day }: { day: PlanDay }) {
  const { t } = useTranslation();
  const steps = day.steps ?? [];
  return (
    <View className="flex-row mt-2" testID={`workout-plan-day-${day.date}`}>
      <Text className={`w-24 text-sm text-text-secondary ${FIGURE}`}>{day.date}</Text>
      <View className="flex-1">
        {day.rest ? (
          <Text className="text-sm text-text-secondary">{t('chat.restDay')}</Text>
        ) : (
          <Text className="text-sm text-text-primary">
            <Text className="font-semibold">{day.workout}</Text>
            <Text className="text-text-secondary">
              {' · '}
              {day.sport}
              {day.duration_min !== undefined ? (
                <Text>
                  {' · '}
                  <Figure>{day.duration_min} min</Figure>
                </Text>
              ) : null}
              {day.intensity ? ` · ${day.intensity}` : ''}
            </Text>
          </Text>
        )}
        {steps.length > 0 ? (
          <Text className="text-sm text-text-secondary mt-0.5">
            <Text className="font-semibold text-text-primary">{t('plan.card.steps')}</Text>{' '}
            {steps.map((step, index) => (
              <Text key={`${step.label}-${index}`}>
                {index > 0 ? '  ·  ' : ''}
                <StepText step={step} />
              </Text>
            ))}
          </Text>
        ) : null}
        {day.template_slug ? (
          <Text className="text-sm text-text-secondary mt-0.5">
            {t('plan.card.template')}{' '}
            <Text className="text-text-primary">{day.template_slug}</Text>
            {day.template_source
              ? ` · ${t(`plan.card.source.${day.template_source}`, { defaultValue: day.template_source })}`
              : ''}
          </Text>
        ) : null}
        {day.fueling ? (
          <Text className="text-sm text-text-secondary mt-0.5">
            <Text className="font-semibold text-text-primary">{t('chat.fuelLabel')}</Text>{' '}
            {fuelParts(day.fueling, t).join(' · ')}
          </Text>
        ) : null}
      </View>
    </View>
  );
}

/**
 * The week's heading: "This week" for the week covering today, "Next week" for
 * the one after it, the Monday's date for any other. The date rides beside the
 * two named weeks so every heading still carries it.
 */
function WeekHeading({
  week,
  index,
  currentIndex,
}: {
  week: PlanWeek;
  index: number;
  currentIndex: number;
}) {
  const { t } = useTranslation();
  const named = week.current
    ? t('plan.card.thisWeek')
    : currentIndex >= 0 && index === currentIndex + 1
      ? t('plan.card.nextWeek')
      : null;
  return (
    <Text className="text-sm">
      <Text className="font-semibold text-text-secondary">
        {named ?? <Sentence text={t('plan.card.weekOf', { date: week.week_start })} figure={week.week_start} />}
      </Text>
      {named ? (
        <Text className="text-text-tertiary">
          {' '}
          <Figure>{week.week_start}</Figure>
        </Text>
      ) : null}
      {week.focus ? <Text className="text-text-secondary"> {week.focus}</Text> : null}
    </Text>
  );
}

/**
 * The card is one outline on the paper: a faint rounded border, no fill, and
 * a single hairline under the header block. Everything under it is separated
 * by spacing alone.
 */
export default function WorkoutPlanCard({ plan }: WorkoutPlanCardProps) {
  const { t } = useTranslation();
  const currentIndex = plan.weeks.findIndex((week) => week.current);

  return (
    <View className="my-2 rounded-xl border border-border-faint overflow-hidden" testID="workout-plan-card">
      <View className="px-4 py-3 border-b border-border-faint">
        <Text className="text-sm font-semibold text-text-primary">{t('app.trainingPlan')}</Text>
        <Text className="text-sm text-text-secondary mt-0.5">
          {t('plan.card.goalRace')}{' '}
          <Text className="font-semibold text-text-primary">{plan.goal_race.name}</Text>
          {' · '}
          <Figure>{plan.goal_race.date}</Figure>
        </Text>
      </View>

      {plan.races !== undefined && plan.races.length > 0 ? (
        <Text className="px-4 mt-3 text-sm text-text-secondary">
          <Text className="font-semibold text-text-primary">{t('plan.card.alsoRacing')}</Text>{' '}
          {plan.races.map((race, index) => (
            <Text key={`${race.name}-${race.date}`}>
              {index > 0 ? '  ·  ' : ''}
              {race.name} <Figure>{race.date}</Figure> ({race.priority})
            </Text>
          ))}
        </Text>
      ) : null}

      <Season plan={plan} />

      {plan.weeks.length > 0 ? (
        <View className="px-4 mt-3">
          {plan.weeks.map((week, index) => (
            <View
              key={week.week_start}
              className={index > 0 ? 'mt-3' : undefined}
              testID={`workout-plan-week-${index + 1}`}
            >
              <WeekHeading week={week} index={index} currentIndex={currentIndex} />
              {week.days.map((day) => (
                <DayRow key={day.date} day={day} />
              ))}
            </View>
          ))}
        </View>
      ) : null}

      {plan.weeks_deferred > 0 ? (
        <Text className="px-4 mt-3 mb-3 text-sm text-text-secondary">
          <Sentence text={t('plan.card.moreWeeks', { count: plan.weeks_deferred })} figure={plan.weeks_deferred} />
        </Text>
      ) : (
        <View className="mb-3" />
      )}
    </View>
  );
}
