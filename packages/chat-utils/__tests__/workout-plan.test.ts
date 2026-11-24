// ABOUTME: Unit tests for the workout-plan card's figures — a step's compact duration and a day's fuelling line
// ABOUTME: Red if a 90 s interval reads "2 min", a stride "0 min", or sodium turns into a prescribed intake

import { describe, expect, it } from 'vitest';
import { clockDuration, fuelParts, stepDuration } from '../src/workout-plan';

/** A translator that shows which key and params it was handed. */
const t = (key: string, params?: Record<string, string | number>) =>
  `${key}(${JSON.stringify(params ?? {})})`;

describe('clockDuration', () => {
  it('reads a duration as a clock, seconds padded, a negative as nothing', () => {
    expect(clockDuration(90)).toBe('1:30');
    expect(clockDuration(600)).toBe('10:00');
    expect(clockDuration(5)).toBe('0:05');
    expect(clockDuration(89.6)).toBe('1:30');
    expect(clockDuration(-3)).toBe('0:00');
  });
});

describe('stepDuration', () => {
  it('prints seconds under a minute so a stride never reads "0 min"', () => {
    expect(stepDuration(20)).toBe('20 s');
    expect(stepDuration(59)).toBe('59 s');
  });

  it('prints whole minutes as "min", never "m", which reads as metres', () => {
    expect(stepDuration(60)).toBe('1 min');
    expect(stepDuration(240)).toBe('4 min');
    expect(stepDuration(1200)).toBe('20 min');
  });

  it('keeps the seconds of a part minute instead of rounding them away', () => {
    expect(stepDuration(90)).toBe('1:30');
    expect(stepDuration(450)).toBe('7:30');
    expect(stepDuration(65)).toBe('1:05');
  });

  it('rounds a fractional second before choosing the form', () => {
    expect(stepDuration(59.6)).toBe('1 min');
    expect(stepDuration(29.4)).toBe('29 s');
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
