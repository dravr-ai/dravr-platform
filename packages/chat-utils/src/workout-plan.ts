// ABOUTME: The pure half of the workout-plan card — a step's compact duration and a day's fuelling fragments
// ABOUTME: Both clients print the plan's figures from here, so a stride or a sodium line cannot read differently on the phone

import type { PlanFueling } from '@pierre/shared-types';

import type { Translate } from './text';

/**
 * A step's duration as the compact figure the steps line carries: whole
 * minutes once a step lasts one, seconds under that so a stride does not
 * read as "0m".
 */
export function stepDuration(seconds: number): string {
  return seconds < 60 ? `${seconds}s` : `${Math.round(seconds / 60)}m`;
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
