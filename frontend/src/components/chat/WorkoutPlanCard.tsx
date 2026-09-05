import { useTranslation } from '@pierre/i18n';
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders a structured-workout plan (builder-coach output) as a readable card.
// ABOUTME: Replaces the raw JSON that builder coaches emit with a scannable weekly plan.

import type {
  WorkoutPlan,
  WorkoutSession,
  WorkoutDay,
  WorkoutRange,
} from '@pierre/shared-types';

type Translate = (key: string, opts?: Record<string, unknown>) => string;

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
 * The fuelling fragments for one session, in display order.
 *
 * A session carries either a full `fueling_protocol` or, on heat sessions, the
 * fluid-only `fluid_protocol`. Sodium is shown as an estimated sweat loss and
 * never as a required intake: the evidence does not support prescribing a mg/h
 * figure, and sodium supplementation does not prevent hyponatremia — fluid
 * volume above sweat rate is what does.
 */
function fuelParts(session: WorkoutSession, t: Translate): string[] {
  const fueling = session.fueling_protocol;
  const fluid = session.fluid_protocol;
  if (fueling) {
    const parts = [
      t('chat.fuelCarbs', { value: fueling.carbs_g_per_h }),
      t('chat.fuelFluid', { value: fueling.fluid_ml_per_h }),
      t('chat.fuelSodiumLoss', { value: fueling.sodium_mg_per_h }),
    ];
    if (fueling.carb_source) {
      parts.push(fueling.carb_source);
    }
    return parts;
  }
  if (fluid) {
    return [
      t('chat.fuelFluid', { value: fluid.fluid_ml_per_h }),
      t('chat.fuelSodiumLoss', { value: fluid.sodium_mg_per_h }),
    ];
  }
  return [];
}

function SessionCell({ session, label }: { session: WorkoutSession; label?: string }) {
  const { t } = useTranslation();
  const lactate = formatRange(session.lactate_target_mmol, ' mmol/L');
  const pacePower = formatRange(session.pace_power_target_pct);
  const fuel = fuelParts(session, t);
  return (
    <div className="mb-1 last:mb-0">
      {label && (
        <span className="mr-1 rounded bg-surface-container-low px-1 text-xs font-medium text-on-surface-variant">
          {label}
        </span>
      )}
      <span className="font-medium text-on-surface">{session.name}</span>
      <span className="text-on-surface-variant">
        {' · '}
        {session.duration_min} min · IF {session.intensity_factor.toFixed(2)} · {session.tss_estimate} TSS
      </span>
      {lactate && <span className="text-on-surface-variant"> · lactate {lactate}</span>}
      {!lactate && pacePower && (
        <span className="text-on-surface-variant"> · {pacePower} of threshold</span>
      )}
      {fuel.length > 0 && (
        <div className="text-xs text-on-surface-variant">
          <span className="font-medium text-on-surface">{t('chat.fuelLabel')}</span>{' '}
          {fuel.join(' · ')}
        </div>
      )}
    </div>
  );
}

function DayRow({ day }: { day: WorkoutDay }) {
  const { t } = useTranslation();
  const rest = !day.session && !day.pm_session;
  return (
    <tr className="border-b ghost-border last:border-0">
      <td className="py-2 pr-3 align-top font-semibold text-on-surface">{day.day}</td>
      <td className="py-2 align-top">
        {rest && <span className="text-on-surface-variant">{t('chat.restDay')}</span>}
        {day.session && (
          <SessionCell session={day.session} label={day.am_pm_split ? 'AM' : undefined} />
        )}
        {day.pm_session && <SessionCell session={day.pm_session} label="PM" />}
      </td>
    </tr>
  );
}

export default function WorkoutPlanCard({ plan }: WorkoutPlanCardProps) {
  const { t } = useTranslation();
  // `compliance` is typed as required but `parseWorkoutPlan` only checks
  // `plan_window` and `weeks`, so a plan without it reaches here and an
  // unguarded dereference blanks the whole app behind the error boundary.
  // Losing the zone strip is a far better outcome than losing the plan.
  const compliance = plan.compliance ?? {};
  const zones = [compliance.z1_pct, compliance.z2_pct, compliance.z3_pct];
  const hasZones = zones.some((z) => typeof z === 'number');

  return (
    <div className="my-2 overflow-hidden rounded-[10px] border ghost-border bg-surface-container-lowest">
      {/* Header — the plan is a data object inside the agent's turn, the one
          place a hairline card survives in the thread (DESIGN.md §5). */}
      <div className="flex flex-wrap items-baseline justify-between gap-2 border-b ghost-border-faint px-3 py-2.5">
        <div className="text-sm font-semibold text-on-surface">
          {t('chat.trainingPlanTitle')}
          <span className="ml-2 font-mono text-xs font-normal text-on-surface-variant">
            {plan.plan_window.start} → {plan.plan_window.end}
          </span>
        </div>
        {plan.lactate_fallback_mode && (
          <span className="text-xs text-on-surface-variant">
            {plan.lactate_fallback_mode.replace('_', '/')} targets
          </span>
        )}
      </div>

      <div className="px-3 py-2.5">
        {/* Rationale */}
        {plan.rationale && (
          <p className="mb-3 text-sm leading-relaxed text-on-surface-variant">{plan.rationale}</p>
        )}

        {/* Compliance summary */}
        <div className="mb-4 flex flex-wrap gap-x-4 gap-y-1 text-xs text-on-surface-variant">
          {hasZones && (
            <span>
              {t('frag.zones')}{' '}
              <span className="font-medium text-on-surface">
                Z1 {compliance.z1_pct ?? 0}% / Z2 {compliance.z2_pct ?? 0}% / Z3 {compliance.z3_pct ?? 0}%
              </span>
            </span>
          )}
          {typeof compliance.weekly_tss_target === 'number' && (
            <span>
              {t('chat.weeklyTss')} <span className="font-medium text-on-surface">{compliance.weekly_tss_target}</span>
            </span>
          )}
          {typeof compliance.polarization_index === 'number' && (
            <span>
              {t('frag.polarization')}{' '}
              <span className="font-medium text-on-surface">
                {compliance.polarization_index.toFixed(2)}
              </span>
            </span>
          )}
          {typeof plan.easy_volume_floor_pct === 'number' && (
            <span>
              {t('chat.easyFloor')} <span className="font-medium text-on-surface">≥{plan.easy_volume_floor_pct}%</span>
            </span>
          )}
        </div>

        {/* Weeks */}
        {plan.weeks.map((week) => (
          <div key={week.week_index} className="mb-4 last:mb-0">
            {plan.weeks.length > 1 && (
              <div className="mb-1 text-xs font-semibold text-on-surface-variant">
                {t('frag.week')} {week.week_index}
                {typeof week.ctl_target === 'number' && (
                  <span className="ml-2 font-normal normal-case">{t('frag.ctlTarget')} {week.ctl_target}</span>
                )}
              </div>
            )}
            {week.gut_training_progression && week.gut_training_progression.length > 0 && (
              <div className="mb-1 text-xs text-on-surface-variant">
                <span className="font-medium text-on-surface">{t('chat.fuelGutTraining')}</span>{' '}
                {week.gut_training_progression.join(' · ')}
              </div>
            )}
            <table className="w-full text-sm">
              <tbody>
                {week.days.map((day) => (
                  <DayRow key={day.day} day={day} />
                ))}
              </tbody>
            </table>
          </div>
        ))}
      </div>
    </div>
  );
}
