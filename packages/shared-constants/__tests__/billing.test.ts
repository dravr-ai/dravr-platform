// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves every billing label — tier, plan row, quota counter, status — is a key all five locales carry
// ABOUTME: An unknown tier reads as no key at all, so the plan page prints the slug rather than a raw key string

import { describe, it, expect } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  INVOICE_STATUS_LABEL_KEY,
  PAYMENT_PROBLEM_STATUSES,
  PLAN_INCLUDED_USAGE_LABEL_KEY,
  PLAN_LIMIT_ROWS,
  PLAN_PER_MONTH_KEY,
  PLAN_UNLIMITED_KEY,
  PLAN_INCLUDED_CUSTOM_KEY,
  PLAN_TIER_LABEL_KEY,
  QUOTA_COUNTER_LABEL_KEY,
  SUBSCRIPTION_STATUS_LABEL_KEY,
  billingLabel,
  hasPaymentProblem,
  invoiceStatusLabelKey,
  planTierLabelKey,
  quotaCounterLabelKey,
  subscriptionStatusLabelKey,
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

/** Every key a billing table names, with the table it came from. */
const BILLING_KEYS: Array<[string, string]> = [
  ...PLAN_LIMIT_ROWS.map((row): [string, string] => ['PLAN_LIMIT_ROWS', row.labelKey]),
  ['PLAN_INCLUDED_USAGE_LABEL_KEY', PLAN_INCLUDED_USAGE_LABEL_KEY],
  ['PLAN_PER_MONTH_KEY', PLAN_PER_MONTH_KEY],
  ['PLAN_UNLIMITED_KEY', PLAN_UNLIMITED_KEY],
  ['PLAN_INCLUDED_CUSTOM_KEY', PLAN_INCLUDED_CUSTOM_KEY],
  ...Object.values(QUOTA_COUNTER_LABEL_KEY).map((key): [string, string] => ['QUOTA_COUNTER_LABEL_KEY', key]),
  ...Object.values(SUBSCRIPTION_STATUS_LABEL_KEY).map((key): [string, string] => ['SUBSCRIPTION_STATUS_LABEL_KEY', key]),
  ...Object.values(INVOICE_STATUS_LABEL_KEY).map((key): [string, string] => ['INVOICE_STATUS_LABEL_KEY', key]),
];

describe('plan rows, quota counters and statuses', () => {
  it('every key resolves to a string in all five locales', () => {
    // The literal-key gate cannot see these: the pages read them through the
    // tables, never as a t('…') literal.
    expect(BILLING_KEYS.length).toBe(4 + 4 + 6 + 8 + 5);
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const [table, key] of BILLING_KEYS) {
        expect(typeof lookup(catalogue, key), `${locale}: ${table} ${key}`).toBe('string');
      }
    }
  });

  it('words an uncapped plan the same on both clients: unlimited rows, custom included usage', () => {
    // The phone once read app.unlimited / app.custom ("Personnalisé") where the
    // web read these keys ("Sur mesure"); both pages now read them from here.
    expect(lookup(fr, PLAN_UNLIMITED_KEY)).toBe('Illimité');
    expect(lookup(fr, PLAN_INCLUDED_CUSTOM_KEY)).toBe('Sur mesure');
    expect(lookup(en, PLAN_INCLUDED_CUSTOM_KEY)).toBe('Custom');
  });

  it('labels every counter the quota endpoint reports', () => {
    // The six counter_type values GET /api/users/me/quota checks, in its order.
    for (const counter of [
      'daily_messages',
      'daily_tokens',
      'weekly_tokens',
      'daily_tool_calls',
      'daily_conversations',
      'active_coaches',
    ]) {
      expect(quotaCounterLabelKey(counter), counter).not.toBeNull();
    }
    expect(lookup(fr, quotaCounterLabelKey('daily_tool_calls') ?? '')).toBe("Appels d'outils par jour");
  });

  it('names the payment-problem statuses, so the dunning sentence never prints a slug', () => {
    for (const status of PAYMENT_PROBLEM_STATUSES) {
      expect(subscriptionStatusLabelKey(status), status).not.toBeNull();
    }
    expect(lookup(fr, subscriptionStatusLabelKey('past_due') ?? '')).toBe('Paiement en retard');
    expect(lookup(fr, invoiceStatusLabelKey('paid') ?? '')).toBe('Payée');
  });

  it('reads a value the tables do not know as the spaced slug, and an inherited name as unknown', () => {
    const t = (key: string): string => `t:${key}`;
    expect(billingLabel('past_due', subscriptionStatusLabelKey, t)).toBe('t:plan.subscriptionStatus.pastDue');
    expect(billingLabel('monthly_tokens', quotaCounterLabelKey, t)).toBe('monthly tokens');
    expect(billingLabel('toString', invoiceStatusLabelKey, t)).toBe('toString');
  });

  it('reads an absent value as a dash, so a degraded payload renders instead of throwing', () => {
    // A subscription answered as `{}` carries no status; the plan page used to
    // call .replace on undefined and unmount the whole dashboard.
    const t = (key: string): string => `t:${key}`;
    expect(billingLabel(undefined, subscriptionStatusLabelKey, t)).toBe('—');
    expect(billingLabel(null, quotaCounterLabelKey, t)).toBe('—');
    expect(billingLabel('', invoiceStatusLabelKey, t)).toBe('—');
  });
});
