// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves every plan tier reads as a corpus key all five locales carry, and names the payment-problem statuses
// ABOUTME: An unknown tier reads as no key at all, so the plan page prints the slug rather than a raw key string

import { describe, it, expect } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  PAYMENT_PROBLEM_STATUSES,
  PLAN_TIER_LABEL_KEY,
  hasPaymentProblem,
  planTierLabelKey,
} from '../src/billing';

const LOCALES = { de, en, es, fr, pt } as const;

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('plan tier labels', () => {
  it('names one key per tier, in the plan namespace', () => {
    expect(PLAN_TIER_LABEL_KEY).toEqual({
      starter: 'plan.starter',
      professional: 'plan.professional',
      enterprise: 'plan.enterprise',
    });
  });

  it('every tier key resolves to a string in all five locales', () => {
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const key of Object.values(PLAN_TIER_LABEL_KEY)) {
        expect(typeof lookup(catalogue, key), `${locale}: ${key}`).toBe('string');
      }
    }
    expect(lookup(en, planTierLabelKey('professional') ?? '')).toBe('Professional');
  });

  it('keeps one key set: the mobile-only app.plan* spelling is gone from every locale', () => {
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const retired of ['app.planStarter', 'app.planProfessional', 'app.planEnterprise']) {
        expect(lookup(catalogue, retired), `${locale}: ${retired}`).toBeUndefined();
      }
    }
  });

  it('reads an unknown tier, or a name the table inherits, as no key', () => {
    expect(planTierLabelKey('platinum')).toBeNull();
    expect(planTierLabelKey('toString')).toBeNull();
    expect(planTierLabelKey('')).toBeNull();
  });
});

describe('payment problems', () => {
  it('names the four statuses that ask the athlete to fix their payment', () => {
    expect([...PAYMENT_PROBLEM_STATUSES].sort()).toEqual([
      'incomplete',
      'incomplete_expired',
      'past_due',
      'unpaid',
    ]);
  });

  it('flags a past-due subscription and nothing that is paid or absent', () => {
    expect(hasPaymentProblem('past_due')).toBe(true);
    expect(hasPaymentProblem('unpaid')).toBe(true);
    expect(hasPaymentProblem('active')).toBe(false);
    expect(hasPaymentProblem('canceled')).toBe(false);
    expect(hasPaymentProblem(undefined)).toBe(false);
    expect(hasPaymentProblem(null)).toBe(false);
  });
});
