// ABOUTME: React Native card that renders a structured-workout plan from a builder-coach reply.
// ABOUTME: Mirrors the web WorkoutPlanCard; replaces raw plan JSON with a scannable weekly view.

import React from 'react';
import { View, Text } from 'react-native';
import type { WorkoutPlan, WorkoutSession, WorkoutDay, WorkoutRange } from '@pierre/shared-types';
import { useThemeColors } from '../../constants/theme';
import { useTranslation } from '@pierre/i18n';

interface WorkoutPlanCardProps {
  plan: WorkoutPlan;
}

function formatRange(range: WorkoutRange | undefined, suffix = ''): string | null {
  if (!range || range.length !== 2) {
    return null;
  }
  return `${range[0]}–${range[1]}${suffix}`;
}

/**
 * The one-line summary under a session.
 *
 * Takes `t` because it is a module-level helper, not a component — the only
 * translated fragment in it is the threshold suffix, and the rest are units
 * that do not vary by locale.
 */
function sessionLine(session: WorkoutSession, t: (key: string, opts?: Record<string, unknown>) => string): string {
  const lactate = formatRange(session.lactate_target_mmol, ' mmol/L');
  const pacePower = formatRange(session.pace_power_target_pct);
  const parts = [
    `${session.duration_min} min`,
    `IF ${session.intensity_factor.toFixed(2)}`,
    `${session.tss_estimate} TSS`,
  ];
  if (lactate) {
    parts.push(`lactate ${lactate}`);
  } else if (pacePower) {
    parts.push(t('app.pctOfThreshold', { value: pacePower }));
  }
  return parts.join(' · ');
}

function SessionRow({ session, label }: { session: WorkoutSession; label?: string }) {
  const { t } = useTranslation();
  return (
    <View className="mb-1">
      <Text className="text-sm text-text-primary">
        {label ? <Text className="text-text-tertiary">{label} </Text> : null}
        <Text className="font-semibold">{session.name}</Text>
      </Text>
      <Text className="text-xs text-text-secondary">{sessionLine(session, t)}</Text>
    </View>
  );
}

export default function WorkoutPlanCard({ plan }: WorkoutPlanCardProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { compliance } = plan;

  const summary: string[] = [];
  if (typeof compliance.z1_pct === 'number') {
    summary.push(`Z1 ${compliance.z1_pct}% / Z2 ${compliance.z2_pct ?? 0}% / Z3 ${compliance.z3_pct ?? 0}%`);
  }
  if (typeof compliance.weekly_tss_target === 'number') {
    summary.push(`${compliance.weekly_tss_target} TSS/wk`);
  }
  if (typeof compliance.polarization_index === 'number') {
    summary.push(`PI ${compliance.polarization_index.toFixed(2)}`);
  }
  if (typeof plan.easy_volume_floor_pct === 'number') {
    summary.push(`easy floor ≥${plan.easy_volume_floor_pct}%`);
  }

  return (
    <View
      className="my-2 rounded-xl overflow-hidden"
      style={{ backgroundColor: colors.tokens.surfaceContainer, borderWidth: 1, borderColor: colors.border.subtle }}
    >
      {/* Header */}
      <View
        className="px-4 py-3 flex-row flex-wrap items-center justify-between"
        style={{ backgroundColor: colors.tokens.surfaceContainerHigh }}
      >
        <Text className="text-sm font-semibold text-text-primary">
          {t('app.trainingPlan')}{' '}
          <Text className="font-normal text-text-secondary">
            {plan.plan_window.start} → {plan.plan_window.end}
          </Text>
        </Text>
        {plan.lactate_fallback_mode ? (
          <Text className="text-xs font-medium text-primary">
            {plan.lactate_fallback_mode.replace('_', '/')} targets
          </Text>
        ) : null}
      </View>

      <View className="px-4 py-3">
        {plan.rationale ? (
          <Text className="text-sm text-text-secondary leading-5 mb-3">{plan.rationale}</Text>
        ) : null}

        {summary.length > 0 ? (
          <Text className="text-xs text-text-tertiary mb-3">{summary.join('  ·  ')}</Text>
        ) : null}

        {plan.weeks.map((week) => (
          <View key={week.week_index} className="mb-3">
            {plan.weeks.length > 1 ? (
              <Text className="text-xs font-semibold uppercase text-text-tertiary mb-1">
                {t('app.week')} {week.week_index}
                {typeof week.ctl_target === 'number' ? `  ·  CTL ${week.ctl_target}` : ''}
              </Text>
            ) : null}
            {week.days.map((day: WorkoutDay) => {
              const rest = !day.session && !day.pm_session;
              return (
                <View
                  key={day.day}
                  className="flex-row py-2"
                  style={{ borderTopWidth: 1, borderTopColor: colors.border.subtle }}
                >
                  <Text className="w-10 text-sm font-semibold text-text-primary">{day.day}</Text>
                  <View className="flex-1">
                    {rest ? <Text className="text-sm text-text-secondary">{t('app.rest')}</Text> : null}
                    {day.session ? (
                      <SessionRow session={day.session} label={day.am_pm_split ? 'AM' : undefined} />
                    ) : null}
                    {day.pm_session ? <SessionRow session={day.pm_session} label="PM" /> : null}
                  </View>
                </View>
              );
            })}
          </View>
        ))}
      </View>
    </View>
  );
}
