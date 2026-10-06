// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the privacy screen's consent-to-AI-use switches — one tap withdraws, one tap gives it back
// ABOUTME: The oauth API is mocked; assertions read the switch value, the calls and the failure alert

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AiConsentSection } from '../AiConsentSection';
import { oauthApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  oauthApi: {
    getProvidersStatus: jest.fn(),
    grantAiConsent: jest.fn(),
    withdrawAiConsent: jest.fn(),
  },
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const grantAiConsent = oauthApi.grantAiConsent as jest.Mock;
const withdrawAiConsent = oauthApi.withdrawAiConsent as jest.Mock;

function card(provider: string, display_name: string, connected: boolean, ai_consent?: boolean) {
  return {
    provider,
    display_name,
    description: '',
    requires_oauth: true,
    connected,
    needs_reauth: false,
    capabilities: [],
    consent_required: ai_consent === false,
    ...(ai_consent === undefined ? {} : { ai_consent }),
  };
}

function renderSection() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AiConsentSection />
    </QueryClientProvider>,
  );
}

describe('AiConsentSection', () => {
  let whoopConsent: boolean;

  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    whoopConsent = true;
    getProvidersStatus.mockImplementation(async () => ({
      providers: [card('whoop', 'WHOOP', true, whoopConsent), card('strava', 'Strava', true)],
    }));
    withdrawAiConsent.mockImplementation(async () => {
      whoopConsent = false;
    });
    grantAiConsent.mockImplementation(async () => {
      whoopConsent = true;
    });
  });

  it('shows a switch only for a connected provider that asks for AI consent', async () => {
    renderSection();
    expect((await screen.findByTestId('ai-consent-switch-whoop')).props.value).toBe(true);
    expect(screen.queryByTestId('ai-consent-switch-strava')).toBeNull();
  });

  it('withdraws in one tap and gives it back in one tap', async () => {
    renderSection();
    fireEvent(await screen.findByTestId('ai-consent-switch-whoop'), 'valueChange', false);
    await waitFor(() => expect(withdrawAiConsent).toHaveBeenCalledWith('whoop'));
    await waitFor(() => expect(screen.getByTestId('ai-consent-switch-whoop').props.value).toBe(false));

    fireEvent(screen.getByTestId('ai-consent-switch-whoop'), 'valueChange', true);
    await waitFor(() => expect(grantAiConsent).toHaveBeenCalledWith('whoop'));
    await waitFor(() => expect(screen.getByTestId('ai-consent-switch-whoop').props.value).toBe(true));
  });

  it('alerts on failure and keeps the server state', async () => {
    withdrawAiConsent.mockRejectedValueOnce(new Error('offline'));
    renderSection();
    fireEvent(await screen.findByTestId('ai-consent-switch-whoop'), 'valueChange', false);
    await waitFor(() => expect(Alert.alert).toHaveBeenCalled());
    expect(screen.getByTestId('ai-consent-switch-whoop').props.value).toBe(true);
  });

  it('renders nothing while no such provider is connected', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card('strava', 'Strava', true)] });
    renderSection();
    await waitFor(() => expect(getProvidersStatus).toHaveBeenCalled());
    expect(screen.queryByTestId('privacy-section-ai-consent')).toBeNull();
  });
});
