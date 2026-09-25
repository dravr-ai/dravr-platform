// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the mobile plan page to the shared billing domain, tier labels and compact formatter
// ABOUTME: A past-due subscription names its plan in the athlete's words; checkout returns to the app's own link

import React from 'react';
import { Linking } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
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
        "Your last payment for the Professional plan didn't go through (status: past_due). Update your payment method to keep your plan.",
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
});
