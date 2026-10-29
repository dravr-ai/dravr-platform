// ABOUTME: WHOOP's owner authorization is stated before its OAuth page opens and the start carries the acceptance
// ABOUTME: Pins the notice dialog, its held Continue, the tos_consent start, and a connected WHOOP that owes it asking again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ProviderConnectionCards from '../ProviderConnectionCards';
import { ThemeProvider } from '../../hooks/useTheme';

const authorizeUrl = vi.fn(
  (provider: string, options?: { tosConsent?: boolean }) =>
    `/api/oauth/authorize/${provider}${options?.tosConsent ? '?tos_consent=true' : ''}`,
);
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: {
    authorizeUrl: (provider: string, options?: { tosConsent?: boolean }) => authorizeUrl(provider, options),
  },
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../SciotteLoginModal', () => ({ default: () => null }));
vi.mock('../IntervalsIcuLinkModal', () => ({ default: () => null }));

const OWNER_AUTHORIZATION =
  'As the owner of this data, I authorize Dravr to store my WHOOP measurements, compute training metrics from them, and provide them to my AI agent. I can revoke this at any time.';

function whoopCard(consent_required: boolean, connected = false) {
  return {
    provider: 'whoop',
    display_name: 'WHOOP',
    requires_oauth: true,
    connected,
    needs_reauth: false,
    capabilities: ['sleep', 'recovery'],
    consent_required,
  };
}

function renderCards() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ProviderConnectionCards />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

async function clickWhoop() {
  const user = userEvent.setup();
  await user.click(await screen.findByRole('button', { name: /WHOOP/ }));
  return user;
}

describe('ProviderConnectionCards — WHOOP owner authorization', () => {
  // The connect goes to WHOOP in this tab, never a new one.
  const assign = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, 'open');
    vi.stubGlobal('location', { ...window.location, assign });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('states the owner authorization before WHOOP opens, and opens nothing until it is ticked', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(true)] });
    renderCards();

    const user = await clickWhoop();

    const dialog = await screen.findByRole('dialog');
    const notice = within(dialog).getByRole('note');
    expect(notice).toHaveTextContent('Before you connect WHOOP');
    expect(notice).toHaveTextContent('GDPR Article 20 and the EU Data Act');
    expect(assign).not.toHaveBeenCalled();

    const cont = within(dialog).getByRole('button', { name: 'Continue' });
    expect(cont).toBeDisabled();
    await user.click(cont);
    expect(assign).not.toHaveBeenCalled();

    await user.click(within(dialog).getByLabelText(OWNER_AUTHORIZATION));
    expect(cont).toBeEnabled();
    await user.click(cont);

    expect(authorizeUrl).toHaveBeenCalledWith('whoop', { tosConsent: true });
    expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/whoop?tos_consent=true');
    expect(window.open).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('starts nothing when the notice is dismissed', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(true)] });
    renderCards();

    const user = await clickWhoop();
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });

  it('goes straight to WHOOP once the account has accepted the notice', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(false)] });
    renderCards();

    await clickWhoop();

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(authorizeUrl).toHaveBeenCalledWith('whoop', { tosConsent: false });
    expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/whoop');
    expect(window.open).not.toHaveBeenCalled();
  });

  it('asks a connected WHOOP that owes the authorization to give it, then reconnects with it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(true, true)] });
    renderCards();

    const state = await screen.findByTestId('provider-authorization-owed-whoop');
    expect(state).toHaveTextContent('Authorize WHOOP to keep syncing');
    expect(screen.queryByText('Connected')).not.toBeInTheDocument();

    const user = await clickWhoop();
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByLabelText(OWNER_AUTHORIZATION));
    await user.click(within(dialog).getByRole('button', { name: 'Continue' }));

    expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/whoop?tos_consent=true');
  });

  it('leaves a connected WHOOP that gave it as Connected, with nothing to do', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(false, true)] });
    renderCards();

    expect(await screen.findByText('Connected')).toBeInTheDocument();
    expect(screen.queryByTestId('provider-authorization-owed-whoop')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /WHOOP/ })).toBeDisabled();
  });
});
