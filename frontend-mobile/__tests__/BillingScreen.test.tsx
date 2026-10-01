// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the mobile plan page to the shared billing domain, tier labels and compact formatter
// ABOUTME: A past-due subscription names its plan in the athlete's words; checkout returns to the app's own link

import React from 'react';
import { Linking } from 'react-native';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockGetSubscription = jest.fn();
const mockGetMyQuota = jest.fn();
const mockListInvoices = jest.fn();
const mockGetPlans = jest.fn();
const mockStartCheckout = jest.fn();
const mockOpenPortal = jest.fn();

jest.mock('../src/services/api', () => ({
  billingApi: {
    getSubscription: () => mockGetSubscription(),
    getMyQuota: () => mockGetMyQuota(),
    listInvoices: () => mockListInvoices(),
    getPlans: () => mockGetPlans(),
    startCheckout: (request: unknown) => mockStartCheckout(request),
    openPortal: (request: unknown) => mockOpenPortal(request),
  },
}));
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'user-1', tier: 'starter' } }),
}));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));
jest.mock('../src/hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { api_tokens: false, billing_header: true }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

import { BillingScreen } from '../src/screens/settings/BillingScreen';

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <BillingScreen />
    </QueryClientProvider>,
  );
}

const professionalPastDue = {
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
};

const starterPlan = {
  tier: 'starter',
  label: 'Starter',
  unlimited: false,
  daily_messages: 50,
  daily_tokens: 500_000,
  monthly_tokens: 5_000_000,
  max_active_agents: 3,
  daily_tool_calls: 200,
  included_usd: null,
};

describe('BillingScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetMyQuota.mockResolvedValue({ tier: 'starter', counters: [] });
    mockListInvoices.mockResolvedValue({ invoices: [] });
    mockGetPlans.mockResolvedValue({ plans: [starterPlan] });
  });

  it('names a past-due plan by its shared tier label and asks for a payment update', async () => {
    mockGetSubscription.mockResolvedValue(professionalPastDue);

    renderScreen();

    expect(await screen.findByText('Payment problem — action needed')).toBeTruthy();
    expect(
      screen.getByText(
        "Your last payment for the Professional plan didn't go through (status: Past due). Update your payment method to keep your plan.",
      ),
    ).toBeTruthy();
    expect(mockGetSubscription).toHaveBeenCalledTimes(1);
  });

  it('prints plan caps with the shared compact formatter', async () => {
    mockGetSubscription.mockResolvedValue(null);

    renderScreen();

    // 500 000 daily tokens and 200 tool calls, as the usage meter prints them.
    expect(await screen.findByText('500.0K')).toBeTruthy();
    expect(screen.getByText('200')).toBeTruthy();
  });

  it('starts a checkout through the shared domain with the app return links', async () => {
    mockGetSubscription.mockResolvedValue(null);
    mockStartCheckout.mockResolvedValue({ checkout_url: 'https://checkout.example/s/1' });
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);

    renderScreen();
    fireEvent.press(await screen.findByText('Upgrade to Professional'));

    await waitFor(() => expect(openURL).toHaveBeenCalledWith('https://checkout.example/s/1'));
    expect(mockStartCheckout).toHaveBeenCalledWith({
      tier: 'professional',
      success_url: 'dravr://billing?upgrade=success',
      cancel_url: 'dravr://billing?upgrade=cancel',
    });
  });

  // The quota counts, the invoice date and its amount followed the device (or
  // a fixed en-US) rather than the language the athlete chose.
  it('writes the quota counts, invoice date and amount in French', async () => {
    // Invoices are read only once a subscription exists.
    mockGetSubscription.mockResolvedValue({ ...professionalPastDue, status: 'active' });
    mockGetMyQuota.mockResolvedValue({
      tier: 'starter',
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
    mockListInvoices.mockResolvedValue({
      invoices: [
        // 2026-01-15T12:00:00Z
        { id: 'in_1', number: 'INV-1', amount_paid: 1250, currency: 'usd', created: 1_768_478_400 },
      ],
    });
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      renderScreen();

      expect(await screen.findByText(/^12\s345 \/ 50\s000$/)).toBeTruthy();
      expect(await screen.findByText('15 janv. 2026')).toBeTruthy();
      expect(screen.getByText(/^12,50\s\$US$/)).toBeTruthy();
      expect(screen.queryByText('1/15/2026')).toBeNull();
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  // The plan rows, the quota labels, the plan price and the subscription
  // status were English literals or raw slugs under French chrome.
  it('writes the plan rows, quota labels, price and status in French', async () => {
    mockGetSubscription.mockResolvedValue(professionalPastDue);
    mockGetPlans.mockResolvedValue({ plans: [{ ...starterPlan, included_usd: 20 }] });
    mockGetMyQuota.mockResolvedValue({
      tier: 'starter',
      counters: [
        {
          counter_type: 'daily_tool_calls',
          current: 12,
          limit: 200,
          warning: false,
          burst_zone: false,
          resets_at: '2026-01-16T00:00:00Z',
        },
      ],
    });
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      renderScreen();

      expect(await screen.findByText('Messages / jour')).toBeTruthy();
      expect(screen.getByText('Jetons / jour')).toBeTruthy();
      expect(screen.getByText('Agents')).toBeTruthy();
      expect(screen.getByText("Appels d'outils / jour")).toBeTruthy();
      expect(screen.getByText('Usage inclus')).toBeTruthy();
      expect(screen.getByText(/^20\s\$US\/mois$/)).toBeTruthy();
      expect(await screen.findByText("Appels d'outils par jour")).toBeTruthy();
      expect(
        screen.getByText(
          "Ton dernier paiement pour le forfait Professional n'a pas abouti (statut : Paiement en retard). Mets à jour ton moyen de paiement pour conserver ton forfait.",
        ),
      ).toBeTruthy();
      expect(screen.getAllByText('Paiement en retard').length).toBeGreaterThan(0);
      expect(screen.queryByText('Messages / day')).toBeNull();
      expect(screen.queryByText('daily tool calls')).toBeNull();
      expect(screen.queryByText(/\/mo$/)).toBeNull();
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });
});
