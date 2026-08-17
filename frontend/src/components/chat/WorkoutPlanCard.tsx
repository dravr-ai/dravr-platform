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

interface WorkoutPlanCardProps {
  plan: WorkoutPlan;
}

function formatRange(range: WorkoutRange | undefined, suffix = ''): string | null {
  if (!range || range.length !== 2) {
    return null;
  }
  return `${range[0]}–${range[1]}${suffix}`;
}

function SessionCell({ session, label }: { session: WorkoutSession; label?: string }) {
  const lactate = formatRange(session.lactate_target_mmol, ' mmol/L');
  const pacePower = formatRange(session.pace_power_target_pct);
  return (
    <div className="mb-1 last:mb-0">
      {label && (
        <span className="mr-1 rounded bg-surface-container-high px-1 text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">
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
    </div>
  );
}

function DayRow({ day }: { day: WorkoutDay }) {
  const rest = !day.session && !day.pm_session;
  return (
    <tr className="border-b border-outline-variant/40 last:border-0">
      <td className="py-2 pr-3 align-top font-semibold text-on-surface">{day.day}</td>
      <td className="py-2 align-top">
        {rest && <span className="text-on-surface-variant">Rest</span>}
        {day.session && (
          <SessionCell session={day.session} label={day.am_pm_split ? 'AM' : undefined} />
        )}
        {day.pm_session && <SessionCell session={day.pm_session} label="PM" />}
      </td>
    </tr>
  );
}

export default function WorkoutPlanCard({ plan }: WorkoutPlanCardProps) {
  // `compliance` is typed as required but `parseWorkoutPlan` only checks
  // `plan_window` and `weeks`, so a plan without it reaches here and an
  // unguarded dereference blanks the whole app behind the error boundary.
  // Losing the zone strip is a far better outcome than losing the plan.
  const compliance = plan.compliance ?? {};
  const zones = [compliance.z1_pct, compliance.z2_pct, compliance.z3_pct];
  const hasZones = zones.some((z) => typeof z === 'number');

  return (
    <div className="my-2 overflow-hidden rounded-xl border border-outline-variant bg-surface-container">
      {/* Header */}
      <div className="flex flex-wrap items-baseline justify-between gap-2 border-b border-outline-variant bg-surface-container-high px-4 py-3">
        <div className="text-sm font-semibold text-on-surface">
          Training plan
          <span className="ml-2 font-normal text-on-surface-variant">
            {plan.plan_window.start} → {plan.plan_window.end}
          </span>
        </div>
        {plan.lactate_fallback_mode && (
          <span className="rounded-full bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">
            {plan.lactate_fallback_mode.replace('_', '/')} targets
          </span>
        )}
      </div>

      <div className="px-4 py-3">
        {/* Rationale */}
        {plan.rationale && (
          <p className="mb-3 text-sm leading-relaxed text-on-surface-variant">{plan.rationale}</p>
        )}

        {/* Compliance summary */}
        <div className="mb-4 flex flex-wrap gap-x-4 gap-y-1 text-xs text-on-surface-variant">
          {hasZones && (
            <span>
              Zones{' '}
              <span className="font-medium text-on-surface">
                Z1 {compliance.z1_pct ?? 0}% / Z2 {compliance.z2_pct ?? 0}% / Z3 {compliance.z3_pct ?? 0}%
              </span>
            </span>
          )}
          {typeof compliance.weekly_tss_target === 'number' && (
            <span>
              Weekly TSS <span className="font-medium text-on-surface">{compliance.weekly_tss_target}</span>
            </span>
          )}
          {typeof compliance.polarization_index === 'number' && (
            <span>
              Polarization{' '}
              <span className="font-medium text-on-surface">
                {compliance.polarization_index.toFixed(2)}
              </span>
            </span>
          )}
          {typeof plan.easy_volume_floor_pct === 'number' && (
            <span>
              Easy floor <span className="font-medium text-on-surface">≥{plan.easy_volume_floor_pct}%</span>
            </span>
          )}
        </div>

        {/* Weeks */}
        {plan.weeks.map((week) => (
          <div key={week.week_index} className="mb-4 last:mb-0">
            {plan.weeks.length > 1 && (
              <div className="mb-1 text-xs font-semibold uppercase tracking-wide text-on-surface-variant">
                Week {week.week_index}
                {typeof week.ctl_target === 'number' && (
                  <span className="ml-2 font-normal normal-case">CTL target {week.ctl_target}</span>
                )}
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
