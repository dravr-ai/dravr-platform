// ABOUTME: Unit tests for the workout-plan card's figures — a step's compact duration and a day's fuelling line
// ABOUTME: Red if a stride reads "0m" or sodium turns into a prescribed intake on either client

import { describe, expect, it } from 'vitest';
import { fuelParts, stepDuration } from '../src/workout-plan';

/** A translator that shows which key and params it was handed. */
const t = (key: string, params?: Record<string, string | number>) =>
  `${key}(${JSON.stringify(params ?? {})})`;

describe('stepDuration', () => {
  it('prints seconds under a minute so a stride never reads "0m"', () => {
    expect(stepDuration(20)).toBe('20s');
    expect(stepDuration(59)).toBe('59s');
  });

  it('rounds to whole minutes from a minute up', () => {
    expect(stepDuration(60)).toBe('1m');
    expect(stepDuration(450)).toBe('8m');
    expect(stepDuration(1200)).toBe('20m');
  });
});

describe('fuelParts', () => {
  it('lists carbohydrate then fluid', () => {
    expect(fuelParts({ carbs_g_per_h: 60, fluid_ml_per_h: 500 }, t)).toEqual([
      'chat.fuelCarbs({"value":60})',
      'chat.fuelFluid({"value":500})',
    ]);
  });

  it('adds sodium as an estimated loss when the plan carries it', () => {
    expect(
      fuelParts({ carbs_g_per_h: 90, fluid_ml_per_h: 750, sodium_mg_per_h: 800 }, t),
    ).toEqual([
      'chat.fuelCarbs({"value":90})',
      'chat.fuelFluid({"value":750})',
      'chat.fuelSodiumLoss({"value":800})',
    ]);
  });
});
