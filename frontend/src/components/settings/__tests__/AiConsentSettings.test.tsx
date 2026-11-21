// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the privacy settings' consent-to-AI-use switches — one tap withdraws, one tap gives it back, health records too
// ABOUTME: The oauth API is mocked at the services barrel; assertions read the switch state, the calls and the status line

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AiConsentSettings from '../AiConsentSettings';

const getProvidersStatus = vi.fn();
const grantAiConsent = vi.fn();
const withdrawAiConsent = vi.fn();
const getAiConsent = vi.fn();

vi.mock('../../../services/api', () => ({
  oauthApi: {
    getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a),
    grantAiConsent: (...a: unknown[]) => grantAiConsent(...a),
    withdrawAiConsent: (...a: unknown[]) => withdrawAiConsent(...a),
    getAiConsent: (...a: unknown[]) => getAiConsent(...a),
  },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

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

function renderPane() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <AiConsentSettings />
    </QueryClientProvider>,
  );
}

describe('AiConsentSettings', () => {
  let whoopConsent: boolean;
  let recordsConsent: boolean;

  beforeEach(() => {
    vi.clearAllMocks();
    whoopConsent = true;
    recordsConsent = false;
    getAiConsent.mockImplementation(async (key: string) => (key === 'sensitive_t2' ? recordsConsent : null));
    getProvidersStatus.mockImplementation(async () => ({
      providers: [
        card('whoop', 'WHOOP', true, whoopConsent),
        card('strava', 'Strava', true),
        card('nolio', 'Nolio', false, true),
      ],
    }));
    withdrawAiConsent.mockImplementation(async (key: string) => {
      if (key === 'sensitive_t2') recordsConsent = false;
      else whoopConsent = false;
    });
    grantAiConsent.mockImplementation(async (key: string) => {
      if (key === 'sensitive_t2') recordsConsent = true;
      else whoopConsent = true;
    });
  });

  it('shows one switch per connected provider that asks for AI consent', async () => {
    renderPane();
    const whoop = await screen.findByTestId('ai-consent-switch-whoop');
    expect(whoop).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('switch', { name: 'Let my AI agent read WHOOP data' })).toBe(whoop);
    expect(screen.queryByTestId('ai-consent-switch-strava')).not.toBeInTheDocument();
    expect(screen.queryByTestId('ai-consent-switch-nolio')).not.toBeInTheDocument();
  });

  it('withdraws in one tap and gives it back in one tap', async () => {
    const user = userEvent.setup();
    renderPane();
    const whoop = await screen.findByTestId('ai-consent-switch-whoop');

    await user.click(whoop);
    expect(withdrawAiConsent).toHaveBeenCalledWith('whoop');
    expect(await screen.findByRole('status')).toHaveTextContent('Your agent no longer reads WHOOP data');
    await vi.waitFor(() =>
      expect(screen.getByTestId('ai-consent-switch-whoop')).toHaveAttribute('aria-checked', 'false'),
    );

    await user.click(screen.getByTestId('ai-consent-switch-whoop'));
    expect(grantAiConsent).toHaveBeenCalledWith('whoop');
    await vi.waitFor(() =>
      expect(screen.getByTestId('ai-consent-switch-whoop')).toHaveAttribute('aria-checked', 'true'),
    );
  });

  it('says so when the change fails, and keeps the server state', async () => {
    withdrawAiConsent.mockRejectedValueOnce(new Error('offline'));
    const user = userEvent.setup();
    renderPane();
    await user.click(await screen.findByTestId('ai-consent-switch-whoop'));
    expect(await screen.findByRole('status')).toHaveTextContent('Could not update your AI consent');
    expect(screen.getByTestId('ai-consent-switch-whoop')).toHaveAttribute('aria-checked', 'true');
  });

  it('shows the health records switch to an account with no such provider connected', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card('strava', 'Strava', true)] });
    renderPane();
    const records = await screen.findByRole('switch', {
      name: 'Let my AI agent read my health records (lab results, clinical records, blood glucose)',
    });
    await vi.waitFor(() => expect(records).toHaveAttribute('aria-checked', 'false'));
    expect(screen.getByText('Sensitive health records')).toBeInTheDocument();
    expect(screen.queryByTestId('ai-consent-switch-whoop')).not.toBeInTheDocument();
    expect(getAiConsent).toHaveBeenCalledWith('sensitive_t2');
  });

  it('allows the health records in one tap and withdraws them in one tap', async () => {
    const user = userEvent.setup();
    renderPane();
    const records = await screen.findByTestId('ai-consent-switch-sensitive_t2');
    await vi.waitFor(() => expect(records).not.toBeDisabled());

    await user.click(records);
    expect(grantAiConsent).toHaveBeenCalledWith('sensitive_t2');
    expect(await screen.findByRole('status')).toHaveTextContent('Your agent reads these records from now on');
    await vi.waitFor(() =>
      expect(screen.getByTestId('ai-consent-switch-sensitive_t2')).toHaveAttribute('aria-checked', 'true'),
    );

    await user.click(screen.getByTestId('ai-consent-switch-sensitive_t2'));
    expect(withdrawAiConsent).toHaveBeenCalledWith('sensitive_t2');
    await vi.waitFor(() =>
      expect(screen.getByTestId('ai-consent-switch-sensitive_t2')).toHaveAttribute('aria-checked', 'false'),
    );
    expect(screen.getByRole('status')).toHaveTextContent('Your agent no longer reads these records');
  });
});
