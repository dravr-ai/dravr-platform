// ABOUTME: The pure half of the workout-plan card — a step's compact duration, a day's fuelling fragments and its one fuel line
// ABOUTME: Both clients print the plan's figures from here, so a stride, a sodium line or a product cannot read differently on the phone

import type { PlanDay, PlanFueling } from '@pierre/shared-types';

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

/**
 * A day's one fuel line: its rates — or, under a medical flag, the statement
 * that a clinician sets them — then the fuel products it names, as
 * "60 g/h carbs · 600 ml/h fluid · Maurten Gel 100, Näak Drink Mix". `null` when the
 * day carries neither, so the card prints no Fuel row.
 */
export function dayFuelLine(
  day: Pick<PlanDay, 'fueling' | 'fueling_withheld' | 'fuel_products'>,
  t: Translate,
): string | null {
  const parts: string[] = [];
  if (day.fueling) {
    parts.push(...fuelParts(day.fueling, t));
  } else if (day.fueling_withheld) {
    parts.push(t('chat.fuelWithheld'));
  }
  const products = (day.fuel_products ?? []).map((product) => product.name);
  if (products.length > 0) {
    parts.push(products.join(', '));
  }
  return parts.length > 0 ? parts.join(' · ') : null;
}
