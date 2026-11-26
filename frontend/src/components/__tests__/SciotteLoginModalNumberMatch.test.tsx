// ABOUTME: A number-match sign-in keeps polling while the number shows, so the athlete's phone tap is seen
// ABOUTME: Pins the re-poll after each poll that answers number_match again, and the connect it ends with,
// ABOUTME: and leaving the number for another method only once the poll in flight has answered

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();
const sciotteSelect2FA = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: (...args: unknown[]) => sciotteSelect2FA(...args),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

const NUMBER_MATCH = { status: 'number_match', number: '93' };

describe('SciotteLoginModal — a number-match sign-in', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('polls again while the number shows, until the tap connects', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    sciotteSelect2FA
      .mockResolvedValueOnce(NUMBER_MATCH)
      .mockResolvedValueOnce(NUMBER_MATCH)
      .mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = vi.fn();
    render(
      <ThemeProvider>
        <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target="garmin" />
      </ThemeProvider>,
    );

    const user = userEvent.setup();
    await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1), { timeout: 8000 });
    expect(sciotteSelect2FA).toHaveBeenCalledTimes(3);
    for (const call of sciotteSelect2FA.mock.calls) expect(call[0]).toBe('poll');
  }, 15000);

  it('leaves the number for another method once the poll in flight answers', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    let answerPoll: (value: unknown) => void = () => undefined;
    sciotteSelect2FA
      .mockImplementationOnce(() => new Promise((resolve) => { answerPoll = resolve; }))
      .mockResolvedValueOnce({
        status: 'two_factor_choice',
        options: [{ id: 'otp', label: 'Get a code from Google Authenticator' }],
      });
    render(
      <ThemeProvider>
        <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={vi.fn()} target="garmin" />
      </ThemeProvider>,
    );

    const user = userEvent.setup();
    await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));

    // The first poll is in flight: asking for another method sends nothing yet,
    // since the poll holds the flow until it answers.
    await vi.waitFor(() => expect(sciotteSelect2FA).toHaveBeenCalledTimes(1));
    await user.click(await screen.findByTestId('sciotte-another-way'));
    expect(sciotteSelect2FA).toHaveBeenCalledTimes(1);

    answerPoll(NUMBER_MATCH);
    expect(
      await screen.findByRole('button', { name: 'Get a code from Google Authenticator' }),
    ).toBeInTheDocument();
    expect(sciotteSelect2FA.mock.calls.map((call) => call[0])).toEqual(['poll', 'another_way']);
  }, 15000);

  it('forgets another method a poll overtook, so a retry keeps polling its new number', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    let failPoll: (reason: unknown) => void = () => undefined;
    sciotteSelect2FA
      .mockImplementationOnce(() => new Promise((_, reject) => { failPoll = reject; }))
      .mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = vi.fn();
    render(
      <ThemeProvider>
        <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target="garmin" />
      </ThemeProvider>,
    );

    const user = userEvent.setup();
    await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));
    await vi.waitFor(() => expect(sciotteSelect2FA).toHaveBeenCalledTimes(1));

    // Asked for while the poll runs, then overtaken: the poll ends the sign-in.
    await user.click(await screen.findByTestId('sciotte-another-way'));
    failPoll(new Error('the sign-in ended'));
    await user.click(await screen.findByRole('button', { name: 'Try Again' }));

    // The retry's number is polled, never left for another method unasked.
    await user.click(await screen.findByRole('button', { name: 'Log In' }));
    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1), { timeout: 8000 });
    expect(sciotteSelect2FA.mock.calls.map((call) => call[0])).toEqual(['poll', 'poll']);
  }, 15000);
});
