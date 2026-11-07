// ABOUTME: The account sign-in as a card's one-time history copy: its intro under the title and its copying line on success
// ABOUTME: Pins that a refusal saying the card now connects through its official authorization reloads the caller's cards
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { i18n } from '@pierre/i18n';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

function renderModal(props: { historyMode?: boolean; onPlanChanged?: () => void } = {}) {
  return render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={vi.fn()} target="garmin" {...props} />
    </ThemeProvider>,
  );
}

async function signIn() {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

describe('SciotteLoginModal — the one-time history copy', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('says under its title that the sign-in copies the history once, and that the copy runs on success', async () => {
    sciotteLogin.mockResolvedValue({ status: 'connected' });
    renderModal({ historyMode: true });

    expect(screen.getByTestId('sciotte-history-intro')).toHaveTextContent(
      i18n.t('providers.history.modalIntro', { provider: 'Garmin' }),
    );
    await signIn();

    expect(await screen.findByText(i18n.t('providers.history.modalCopying'))).toBeInTheDocument();
    expect(screen.queryByText(i18n.t('shell.sciotteConnectedDataReady', { provider: 'Garmin' }))).not.toBeInTheDocument();
  });

  it('keeps the connection words outside the history copy', async () => {
    sciotteLogin.mockResolvedValue({ status: 'connected' });
    renderModal();

    expect(screen.queryByTestId('sciotte-history-intro')).not.toBeInTheDocument();
    await signIn();

    expect(await screen.findByText(i18n.t('shell.sciotteConnectedDataReady', { provider: 'Garmin' }))).toBeInTheDocument();
    expect(screen.queryByText(i18n.t('providers.history.modalCopying'))).not.toBeInTheDocument();
  });

  it('words an oauth_required refusal and reloads the cards, so the next tap takes the authorization', async () => {
    sciotteLogin.mockRejectedValue({
      response: { status: 403, data: { code: 'PermissionDenied', message: 'x', details: { reason: 'oauth_required' } } },
    });
    const onPlanChanged = vi.fn();
    renderModal({ onPlanChanged });

    await signIn();

    expect(
      await screen.findByText(i18n.t('hosted.common.signInOauthRequired').split('{0}').join('Garmin')),
    ).toBeInTheDocument();
    expect(onPlanChanged).toHaveBeenCalledTimes(1);
  });

  it('reloads nothing on any other refusal', async () => {
    sciotteLogin.mockRejectedValue({
      response: { status: 401, data: { code: 'AuthInvalid', message: 'bad', details: { reason: 'rejected' } } },
    });
    const onPlanChanged = vi.fn();
    renderModal({ onPlanChanged });

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
    expect(onPlanChanged).not.toHaveBeenCalled();
  });
});
