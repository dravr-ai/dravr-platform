// ABOUTME: React Native card for the workout_plan block — the saved plan's season and fortnight, mirroring the web card.
// ABOUTME: Reads the projection the platform built from the rows save_training_plan wrote; the agent's prose never feeds it.

import React from 'react';
import { View, Text } from 'react-native';
import type {
  PlanDay,
  PlanFueling,
  PlanPhase,
  PlanStep,
  PlanWeek,
  WorkoutPlan,
} from '@pierre/shared-types';

import { useThemeColors } from '../../constants/theme';
import { useTranslation } from '@pierre/i18n';

type Translate = (key: string, opts?: Record<string, unknown>) => string;

interface WorkoutPlanCardProps {
  plan: WorkoutPlan;
}

/**
 * A step's duration as the compact figure the steps line carries: whole
 * minutes once a step lasts one, seconds under that so a stride does not read
 * as "0m".
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
 * The fuelling fragments for one day, in display order. Sodium is shown as an
 * estimated sweat loss and never as a required intake: the evidence does not
 * support prescribing a mg/h figure.
 */
function fuelParts(fueling: PlanFueling, t: Translate): string[] {
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
function phaseSummary(phase: PlanPhase, t: Translate): string {
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
  const colors = useThemeColors();
  return (
    <View
      className="flex-row items-center mr-1 mb-1 px-2 py-1 rounded-md"
      style={{ backgroundColor: colors.tokens.surfaceContainerHigh }}
    >
      <Text className="text-xs font-semibold text-text-primary">
        {t(`plan.card.phase.${phase.kind}`, { defaultValue: phase.kind })}
      </Text>
      <Text className="text-xs text-text-secondary"> {t('plan.card.weeksShort', { count: phase.weeks })}</Text>
      {phase.current ? (
        <View
          className="ml-1 px-1.5 rounded-full"
          style={{ backgroundColor: colors.tokens.primaryContainer }}
        >
          <Text className="text-xs font-semibold" style={{ color: colors.tokens.onPrimaryContainer }}>
            {t('plan.card.now')}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

/**
 * The season: the flavour with who chose it, then the phases in order as a
 * compact row, then the current phase's rhythm.
 */
function Season({ plan }: { plan: WorkoutPlan }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { flavour, phases } = plan;
  const current = phases.find((phase) => phase.current);
  if (!flavour && phases.length === 0) {
    return null;
  }
  return (
    <View
      className="px-4 py-3"
      style={{ borderBottomWidth: 1, borderBottomColor: colors.border.subtle }}
    >
      {flavour ? (
        <Text className="text-sm mb-2">
          <Text className="text-text-secondary">{t('plan.card.approach')} </Text>
          <Text className="font-semibold text-text-primary">{flavour.label}</Text>
          <Text className="text-xs text-text-secondary">
            {' · '}
            {t(`plan.card.selectedBy.${flavour.selected_by}`, { defaultValue: flavour.selected_by })}
          </Text>
        </Text>
      ) : null}
      {phases.length > 0 ? (
        <>
          <Text className="text-xs text-text-secondary mb-1">
            {t('plan.card.season')}
            {plan.season_start && plan.season_end ? `  ${plan.season_start} → ${plan.season_end}` : ''}
          </Text>
          <View className="flex-row flex-wrap">
            {phases.map((phase) => (
              <PhaseSegment key={`${phase.kind}-${phase.start}`} phase={phase} />
            ))}
          </View>
          {current ? (
            <Text className="text-xs text-text-secondary mt-1">{phaseSummary(current, t)}</Text>
          ) : null}
        </>
      ) : null}
    </View>
  );
}

/**
 * One day of the fortnight: the date, the sport, and the session — its text,
 * duration and intensity, then the steps, the template and the fuel it
 * carries. A rest day is the one word.
 */
function DayRow({ day }: { day: PlanDay }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const meta = [day.duration_min !== undefined ? `${day.duration_min} min` : null, day.intensity].filter(
    (part): part is string => typeof part === 'string' && part.length > 0,
  );
  const steps = day.steps ?? [];
  return (
    <View
      className="flex-row py-2"
      style={{ borderTopWidth: 1, borderTopColor: colors.border.subtle }}
    >
      <Text className="w-20 text-xs text-text-secondary">{day.date}</Text>
      <View className="flex-1">
        {day.rest ? (
          <Text className="text-sm text-text-secondary">{t('chat.restDay')}</Text>
        ) : (
          <Text className="text-sm text-text-primary">
            <Text className="font-semibold">{day.workout}</Text>
            <Text className="text-text-secondary">
              {' · '}
              {day.sport}
              {meta.length > 0 ? ` · ${meta.join(' · ')}` : ''}
            </Text>
          </Text>
        )}
        {steps.length > 0 ? (
          <Text className="text-xs text-text-secondary mt-0.5">
            <Text className="font-semibold text-text-primary">{t('plan.card.steps')}</Text>{' '}
            {steps.map(stepText).join('  ·  ')}
          </Text>
        ) : null}
        {day.template_slug ? (
          <Text className="text-xs text-text-secondary mt-0.5">
            {t('plan.card.template')}{' '}
            <Text className="text-text-primary">{day.template_slug}</Text>
            {day.template_source
              ? ` · ${t(`plan.card.source.${day.template_source}`, { defaultValue: day.template_source })}`
              : ''}
          </Text>
        ) : null}
        {day.fueling ? (
          <Text className="text-xs text-text-secondary mt-0.5">
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
    <Text className="text-xs mb-1">
      <Text className="font-semibold text-text-secondary">
        {named ?? t('plan.card.weekOf', { date: week.week_start })}
      </Text>
      {named ? <Text className="text-text-tertiary"> {week.week_start}</Text> : null}
      {week.focus ? <Text className="text-text-secondary"> {week.focus}</Text> : null}
    </Text>
  );
}

export default function WorkoutPlanCard({ plan }: WorkoutPlanCardProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const currentIndex = plan.weeks.findIndex((week) => week.current);

  return (
    <View
      className="my-2 rounded-xl overflow-hidden"
      style={{
        backgroundColor: colors.tokens.surfaceContainer,
        borderWidth: 1,
        borderColor: colors.border.subtle,
      }}
    >
      <View
        className="px-4 py-3"
        style={{ backgroundColor: colors.tokens.surfaceContainerHigh }}
      >
        <Text className="text-sm font-semibold text-text-primary">{t('app.trainingPlan')}</Text>
        <Text className="text-xs text-text-secondary mt-0.5">
          {t('plan.card.goalRace')}{' '}
          <Text className="font-semibold text-text-primary">{plan.goal_race.name}</Text>
          {' · '}
          {plan.goal_race.date}
        </Text>
      </View>

      {plan.races !== undefined && plan.races.length > 0 ? (
        <Text
          className="px-4 py-2 text-xs text-text-secondary"
          style={{ borderBottomWidth: 1, borderBottomColor: colors.border.subtle }}
        >
          <Text className="font-semibold text-text-primary">{t('plan.card.alsoRacing')}</Text>{' '}
          {plan.races
            .map((race) => `${race.name} ${race.date} (${race.priority})`)
            .join('  ·  ')}
        </Text>
      ) : null}

      <Season plan={plan} />

      {plan.weeks.length > 0 ? (
        <View className="px-4 py-3">
          {plan.weeks.map((week, index) => (
            <View key={week.week_start} className="mb-3">
              <WeekHeading week={week} index={index} currentIndex={currentIndex} />
              {week.days.map((day) => (
                <DayRow key={day.date} day={day} />
              ))}
            </View>
          ))}
        </View>
      ) : null}

      {plan.weeks_deferred > 0 ? (
        <Text
          className="px-4 py-2 text-xs text-text-secondary"
          style={{ borderTopWidth: 1, borderTopColor: colors.border.subtle }}
        >
          {t('plan.card.moreWeeks', { count: plan.weeks_deferred })}
        </Text>
      ) : null}
    </View>
  );
}
