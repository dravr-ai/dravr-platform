// ABOUTME: Unit tests for the workout-plan card's figures — a step's compact duration and a day's fuelling line
// ABOUTME: Red if a stride reads "0m", sodium turns into a prescribed intake, or a day's fuel splits into two rows

import { describe, expect, it } from 'vitest';
import { dayFuelLine, fuelParts, stepDuration } from '../src/workout-plan';

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

describe('dayFuelLine', () => {
  const products = [
    { product_id: 'a', name: 'Maurten Gel 100' },
    { product_id: 'b', name: 'Näak Drink Mix' },
  ];

  it('puts the rates and the products on one line', () => {
    expect(
      dayFuelLine({ fueling: { carbs_g_per_h: 60, fluid_ml_per_h: 600 }, fuel_products: products }, t),
    ).toBe('chat.fuelCarbs({"value":60}) · chat.fuelFluid({"value":600}) · Maurten Gel 100, Näak Drink Mix');
  });

  it('names the products after the withheld statement under a medical flag', () => {
    expect(
      dayFuelLine(
        {
          fueling_withheld: { reason: 'medical_flag', set_by: 'clinician', note: 'A clinician sets these.' },
          fuel_products: products,
        },
        t,
      ),
    ).toBe('chat.fuelWithheld({}) · Maurten Gel 100, Näak Drink Mix');
  });

  it('prints the rates alone, the products alone, or nothing', () => {
    expect(dayFuelLine({ fueling: { carbs_g_per_h: 90, fluid_ml_per_h: 750 } }, t)).toBe(
      'chat.fuelCarbs({"value":90}) · chat.fuelFluid({"value":750})',
    );
    expect(dayFuelLine({ fuel_products: products }, t)).toBe('Maurten Gel 100, Näak Drink Mix');
    expect(dayFuelLine({ fuel_products: [] }, t)).toBeNull();
    expect(dayFuelLine({}, t)).toBeNull();
  });
});
