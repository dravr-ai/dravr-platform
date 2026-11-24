// ABOUTME: The pure half of the workout-plan card — a step's compact duration and a day's fuelling fragments
// ABOUTME: Both clients print the plan's figures from here, so a stride or a sodium line cannot read differently on the phone

import type { PlanFueling } from '@pierre/shared-types';

import type { Translate } from './text';

/**
 * A step's duration as the compact figure the steps line carries: seconds
 * under a minute ("30 s") so a stride does not read as "0 min", whole minutes
 * as "4 min", and anything else as "1:30" so a 90 s interval is not rounded
 * up to two minutes. "min", never "m", because the same card prints distances
 * in metres.
 */
export function stepDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return `${total} s`;
  return total % 60 === 0 ? `${total / 60} min` : clockDuration(total);
}

/**
 * A duration as a clock reads it, `m:ss` (`90` → `1:30`, `600` → `10:00`): the
 * steps line's odd minutes and the day editor's step field print the same.
 */
export function clockDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

/**
 * The fuelling fragments for one day, in display order. Sodium is shown as
 * an estimated sweat loss and never as a required intake: the evidence does
 * not support prescribing a mg/h figure.
 */
export function fuelParts(fueling: PlanFueling, t: Translate): string[] {
  const parts = [
    t('chat.fuelCarbs', { value: fueling.carbs_g_per_h }),
    t('chat.fuelFluid', { value: fueling.fluid_ml_per_h }),
  ];
  if (fueling.sodium_mg_per_h !== undefined) {
    parts.push(t('chat.fuelSodiumLoss', { value: fueling.sodium_mg_per_h }));
  }
  return parts;
}
