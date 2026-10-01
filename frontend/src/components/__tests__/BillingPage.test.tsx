// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the billing page's dates, quota counts and invoice amounts to the language the athlete chose
// ABOUTME: They followed the browser locale (or a fixed en-US), so French chrome printed 1/15/2026 and $12.50

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import BillingPage from '../BillingPage';

const billing = vi.hoisted(() => ({
  getSubscription: vi.fn(),
  getMyQuota: vi.fn(),
  listInvoices: vi.fn(),
  getPlans: vi.fn(),
  startCheckout: vi.fn(),
  openPortal: vi.fn(),
}));

vi.mock('../../services/api', () => ({ billingApi: billing }));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'user-1', email: 'a@b.c', role: 'user', tier: 'professional' } }),
}));
vi.mock('../../hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { billing_header: true }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <BillingPage />
    </QueryClientProvider>,
  );
}

describe('BillingPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    billing.getSubscription.mockResolvedValue({
      id: 'sub-1',
      tenant_id: 't-1',
      user_id: 'user-1',
      provider: 'stripe',
      provider_customer_id: 'cus_1',
      provider_subscription_id: 'sub_stripe_1',
      status: 'active',
      plan_tier: 'professional',
      current_period_start: null,
      // Midday UTC, so the calendar day is the same in every runner's zone.
      current_period_end: '2026-02-15T12:00:00Z',
      cancel_at_period_end: false,
    });
    billing.getMyQuota.mockResolvedValue({
      tier: 'professional',
      counters: [
        {
          counter_type: 'daily_tokens',
          current: 12_345,
          limit: 50_000,
          warning: false,
          burst_zone: false,
          resets_at: '2026-01-16T00:00:00Z',
        },
      ],
    });
    billing.listInvoices.mockResolvedValue({
      invoices: [
        // 2026-01-15T12:00:00Z
        { id: 'in_1', number: 'INV-1', amount_paid: 1250, currency: 'usd', status: 'paid', created: 1_768_478_400 },
      ],
    });
    billing.getPlans.mockResolvedValue({ plans: [] });
  });

  it('writes the period end, the quota counts and the invoice in French', async () => {
    await i18n.changeLanguage('fr');
    try {
      renderPage();

      expect(await screen.findByText('15 févr. 2026')).toBeInTheDocument();
      expect(await screen.findByText(/^12\s345 \/ 50\s000$/)).toBeInTheDocument();
      expect(await screen.findByText('15 janv. 2026')).toBeInTheDocument();
      expect(screen.getByText(/^12,50\s\$US$/)).toBeInTheDocument();
      expect(screen.queryByText('2/15/2026')).not.toBeInTheDocument();
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('writes the same figures in English for an English athlete', async () => {
    renderPage();

    expect(await screen.findByText('Feb 15, 2026')).toBeInTheDocument();
    expect(await screen.findByText('12,345 / 50,000')).toBeInTheDocument();
    expect(await screen.findByText('$12.50')).toBeInTheDocument();
  });

  // The plan rows, the quota label, the plan price, the subscription and
  // invoice statuses and the dunning sentence were English literals or raw
  // slugs under French chrome.
  it('writes the plan rows, quota labels, price, statuses and dunning sentence in French', async () => {
    billing.getSubscription.mockResolvedValue({
      id: 'sub-1',
      tenant_id: 't-1',
      user_id: 'user-1',
      provider: 'stripe',
      provider_customer_id: 'cus_1',
      provider_subscription_id: 'sub_stripe_1',
      status: 'past_due',
      plan_tier: 'professional',
      current_period_start: null,
      current_period_end: null,
      cancel_at_period_end: false,
    });
    billing.getPlans.mockResolvedValue({
      plans: [
        {
          tier: 'professional',
          label: 'Professional',
          unlimited: false,
          daily_messages: 500,
          daily_tokens: 500_000,
          monthly_tokens: 5_000_000,
          max_active_agents: 10,
          daily_tool_calls: 2_000,
          included_usd: 20,
        },
      ],
    });
    await i18n.changeLanguage('fr');
    try {
      renderPage();

      expect(await screen.findByText('Messages / jour')).toBeInTheDocument();
      expect(screen.getByText('Jetons / jour')).toBeInTheDocument();
      expect(screen.getByText('Agents')).toBeInTheDocument();
      expect(screen.getByText("Appels d'outils / jour")).toBeInTheDocument();
      expect(screen.getByText('Usage inclus')).toBeInTheDocument();
      expect(screen.getByText(/^20\s\$US\/mois$/)).toBeInTheDocument();
      expect(await screen.findByText('Jetons par jour')).toBeInTheDocument();
      expect(await screen.findByText('Payée')).toBeInTheDocument();
      expect(screen.getByText('Non')).toBeInTheDocument();
      expect(
        screen.getByText(
          "Ton dernier paiement pour le forfait Professional n'a pas abouti (statut : Paiement en retard). Mets à jour ton moyen de paiement pour conserver ton forfait.",
        ),
      ).toBeInTheDocument();
      expect(screen.getAllByText('Paiement en retard').length).toBeGreaterThan(0);
      expect(screen.queryByText('Messages / day')).not.toBeInTheDocument();
      expect(screen.queryByText(/daily tokens/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/didn.t go through/)).not.toBeInTheDocument();
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
