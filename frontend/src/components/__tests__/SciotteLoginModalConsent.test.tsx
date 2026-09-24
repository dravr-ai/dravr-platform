// ABOUTME: The credential login states the TrainingPeaks notice before the credentials and holds Log In until accepted
// ABOUTME: Pins the login body each target sends, and the Boreal inks the notice, the Log In button and the provider mark take
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { SciotteTarget } from '@pierre/shared-types';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteConfig: vi.fn().mockResolvedValue({ login_timeout_secs: 240 }),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

function renderModal(target: SciotteTarget, consentRequired: boolean) {
  return render(
    <ThemeProvider>
      <SciotteLoginModal
        isOpen
        onClose={vi.fn()}
        onConnected={vi.fn()}
        target={target}
        consentRequired={consentRequired}
      />
    </ThemeProvider>,
  );
}

async function fillCredentials(identifier: string, value = 'coach-account') {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(identifier), value);
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  return user;
}

describe('SciotteLoginModal — the TrainingPeaks notice', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'stop here' });
  });

  it('states the notice before the credentials and sends nothing until it is accepted', async () => {
    renderModal('trainingpeaks', true);

    const notice = screen.getByRole('note');
    expect(notice).toHaveTextContent('Before you connect TrainingPeaks');
    expect(notice).toHaveTextContent('Terms of Use (section 13)');
    expect(notice).toHaveTextContent(
      'Dravr also uses it to read the calendars of the athletes who confirm a link in a group you oversee',
    );
    // The notice comes first: the credentials form follows it in the page.
    const form = screen.getByLabelText('Username').closest('form');
    expect(notice.compareDocumentPosition(form as Node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    const user = await fillCredentials('Username');
    const logIn = screen.getByRole('button', { name: 'Log In' });
    expect(logIn).toBeDisabled();
    await user.click(logIn);
    expect(sciotteLogin).not.toHaveBeenCalled();

    await user.click(
      screen.getByLabelText('I understand that TrainingPeaks could suspend my account, and I accept that risk.'),
    );
    expect(logIn).toBeEnabled();
    await user.click(logIn);

    expect(sciotteLogin).toHaveBeenCalledTimes(1);
    expect(sciotteLogin).toHaveBeenCalledWith({
      email: 'coach-account',
      password: 'not-a-real-password',
      method: 'email',
      target: 'trainingpeaks',
      tos_consent: true,
    });
  });

  it('asks TrainingPeaks for a username, not an email', () => {
    renderModal('trainingpeaks', true);
    const identifier = screen.getByLabelText('Username');
    expect(identifier).toHaveAttribute('type', 'text');
    expect(identifier).toHaveAttribute('autocomplete', 'username');
  });

  it('goes straight to the credentials once the account has accepted the notice', async () => {
    renderModal('trainingpeaks', false);
    expect(screen.queryByRole('note')).not.toBeInTheDocument();

    const user = await fillCredentials('Username');
    await user.click(screen.getByRole('button', { name: 'Log In' }));

    expect(sciotteLogin).toHaveBeenCalledWith({
      email: 'coach-account',
      password: 'not-a-real-password',
      method: 'email',
      target: 'trainingpeaks',
    });
  });

  it('never shows the notice for Garmin, whatever the flag says', async () => {
    renderModal('garmin', true);
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Email')).toHaveAttribute('type', 'email');

    const user = await fillCredentials('Email', 'athlete@example.test');
    await user.click(screen.getByRole('button', { name: 'Log In' }));

    expect(sciotteLogin).toHaveBeenCalledWith({
      email: 'athlete@example.test',
      password: 'not-a-real-password',
      method: 'email',
      target: 'garmin',
    });
  });
});

describe('SciotteLoginModal — Boreal inks', () => {
  afterEach(() => {
    localStorage.removeItem('dravr.theme');
    document.documentElement.classList.remove('dark');
  });

  it('writes the notice in the amber tint\'s own ink, not the body ink', () => {
    renderModal('trainingpeaks', true);

    const notice = screen.getByRole('note');
    expect(notice).toHaveClass('bg-warning/10');
    const title = screen.getByText('Before you connect TrainingPeaks');
    expect(title).toHaveClass('text-on-warning-container');
    expect(title).not.toHaveClass('text-on-surface');
    const body = notice.querySelectorAll('p')[1];
    expect(body).toHaveClass('text-on-warning-container');
    expect(body.className).not.toMatch(/text-on-surface/);
  });

  it('makes Log In the Boreal primary button, full width, disabled until the form is complete', () => {
    renderModal('garmin', false);

    const logIn = screen.getByRole('button', { name: 'Log In' });
    expect(logIn).toHaveClass('btn-primary', 'btn-lg', 'w-full');
    expect(logIn).toHaveAttribute('type', 'submit');
    expect(logIn).toBeDisabled();
    expect(logIn.className).not.toMatch(/gradient|from-|to-warning|\/40/);
  });

  it('dims the page behind it with the Boreal scrim, not stock black', () => {
    renderModal('garmin', false);

    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveClass('bg-scrim/60');
    expect(dialog.className).not.toMatch(/bg-black|backdrop-blur/);
  });

  it('draws the TrainingPeaks mark in its blue on light and in body ink on dark', () => {
    localStorage.setItem('dravr.theme', 'light');
    const { unmount } = renderModal('trainingpeaks', false);
    expect(screen.getByTestId('sciotte-provider-mark')).toHaveStyle({ color: 'rgb(0, 86, 149)' });
    unmount();

    localStorage.setItem('dravr.theme', 'dark');
    renderModal('trainingpeaks', false);
    const mark = screen.getByTestId('sciotte-provider-mark');
    expect(mark).toHaveClass('text-on-surface');
    expect(mark.style.color).toBe('');
  });

  it('draws the Strava mark in Strava orange in both schemes, on a surface tile rather than the warning token', () => {
    for (const scheme of ['light', 'dark']) {
      localStorage.setItem('dravr.theme', scheme);
      const { unmount } = renderModal('strava', false);
      const mark = screen.getByTestId('sciotte-provider-mark');
      expect(mark).toHaveStyle({ color: 'rgb(252, 76, 2)' });
      expect(mark).toHaveClass('bg-surface-container-lowest');
      expect(mark).not.toHaveClass('bg-warning');
      unmount();
    }
  });
});
