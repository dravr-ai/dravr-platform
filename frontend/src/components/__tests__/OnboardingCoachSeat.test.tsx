// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the coach-seat step: two plain rows, one link to ask, and only what the server did
// ABOUTME: TrainingPeaks granted, a dated request, a connection that produced nothing — read back, never inferred from the tap

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { COACH_SEAT_CHECK_WINDOW_MS } from '@pierre/shared-constants';
import OnboardingCoachSeat from '../OnboardingCoachSeat';
import { ThemeProvider } from '../../hooks/useTheme';

const { getProvidersStatus, getCoachAccessRequest, requestCoachAccess, getSession, updateUser, auth } =
  vi.hoisted(() => ({
    getProvidersStatus: vi.fn(),
    getCoachAccessRequest: vi.fn(),
    requestCoachAccess: vi.fn(),
    getSession: vi.fn(),
    updateUser: vi.fn(),
    auth: { managesRoster: false },
  }));

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: { authorizeUrl: (provider: string) => `/api/oauth/authorize/${provider}` },
  userApi: {
    getCoachAccessRequest: (...args: unknown[]) => getCoachAccessRequest(...args),
    requestCoachAccess: (...args: unknown[]) => requestCoachAccess(...args),
  },
  authApi: { getSession: (...args: unknown[]) => getSession(...args) },
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
// The session user as the auth context holds it: a patch re-renders the step.
vi.mock('../../hooks/useAuth', async () => {
  const { useState } = await import('react');
  return {
    useAuth: () => {
      const [managesRoster, setManagesRoster] = useState(auth.managesRoster);
      return {
        user: { manages_roster: managesRoster },
        updateUser: (patch: { manages_roster?: boolean }) => {
          updateUser(patch);
          if (patch.manages_roster !== undefined) setManagesRoster(patch.manages_roster);
        },
        logout: vi.fn(),
      };
    },
  };
});

function card(provider: string, display_name: string, extra: Record<string, unknown> = {}) {
  return {
    provider,
    display_name,
    description: `${display_name} activity data`,
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    ...extra,
  };
}

const pending = {
  id: 'r-1',
  user_id: 'u-1',
  group_id: null,
  group_tenant_id: null,
  status: 'pending',
  created_at: '2026-11-12T19:32:00Z',
  decided_at: null,
  decided_by: null,
};

function served(overrides: Record<string, Record<string, unknown>> = {}) {
  return {
    providers: [
      card('sciotte', 'Strava'),
      card('intervals_icu', 'Intervals.icu', overrides.intervals_icu),
      card('sciotte_trainingpeaks', 'TrainingPeaks', overrides.sciotte_trainingpeaks),
    ],
  };
}

function renderSeat() {
  const onComplete = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <OnboardingCoachSeat onComplete={onComplete} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { onComplete };
}

describe('OnboardingCoachSeat', () => {
  beforeEach(() => {
    auth.managesRoster = false;
    getProvidersStatus.mockReset().mockResolvedValue(served());
    getCoachAccessRequest.mockReset().mockResolvedValue({ request: null });
    requestCoachAccess.mockReset();
    getSession.mockReset().mockResolvedValue({ user: { manages_roster: false } });
    updateUser.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('offers TrainingPeaks then Intervals.icu as plain rows, the link to ask, and a way past', async () => {
    const { onComplete } = renderSeat();

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Get your coach seat'); // humanCoach.seatHeading
    expect(screen.getByText('Link your coach account, or ask the team for it.')).toBeInTheDocument(); // humanCoach.seatIntro
    await screen.findByText('TrainingPeaks');
    const rows = screen.getAllByTestId(/^provider-card-/).map((row) => row.dataset.testid);
    expect(rows).toEqual(['provider-card-sciotte_trainingpeaks', 'provider-card-intervals_icu']);
    // Plain rows: no activity-data line, no import, no Strava.
    expect(screen.queryByText('TrainingPeaks activity data')).not.toBeInTheDocument();
    expect(screen.queryByText('Strava')).not.toBeInTheDocument();
    expect(screen.queryByText(/Import your own data/)).not.toBeInTheDocument();
    // Nothing claimed before anything happened.
    expect(screen.queryByTestId('coach-seat-status')).not.toBeInTheDocument();
    expect(screen.getByTestId('coach-seat-ask')).toHaveTextContent('I use neither →');
    expect(screen.queryByTestId('coach-seat-continue')).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(requestCoachAccess).not.toHaveBeenCalled();
  });

  it('« I use neither » files a request with no group, then shows it dated, with Continue', async () => {
    requestCoachAccess.mockResolvedValue({ request: pending });
    const { onComplete } = renderSeat();
    await screen.findByText('TrainingPeaks');

    await userEvent.click(screen.getByTestId('coach-seat-ask'));

    expect(requestCoachAccess).toHaveBeenCalledWith();
    const status = await screen.findByTestId('coach-seat-status');
    expect(status).toHaveAttribute('data-state', 'requested');
    expect(status).toHaveTextContent('Request sent · usually within 24 h');
    expect(status).toHaveTextContent(/Sent Nov 12, 2026/);
    // Only what ships: no promise of an email or an in-app notice.
    expect(status).not.toHaveTextContent(/email/i);
    await userEvent.click(screen.getByTestId('coach-seat-continue'));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('a request already waiting is shown as it stands, never filed twice', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: pending });
    renderSeat();
    const status = await screen.findByTestId('coach-seat-status');
    expect(status).toHaveAttribute('data-state', 'requested');
    expect(screen.queryByTestId('coach-seat-ask')).not.toBeInTheDocument();
  });

  it('TrainingPeaks granted: reads the seat back from the session and says the coach coaches their groups', async () => {
    getProvidersStatus.mockResolvedValue(
      served({ sciotte_trainingpeaks: { connected: true, account_role: 'coach' } }),
    );
    getSession.mockResolvedValue({ user: { manages_roster: true } });
    renderSeat();

    await waitFor(() => expect(updateUser).toHaveBeenCalledWith({ manages_roster: true }));
    expect(await screen.findByTestId('coach-seat-status')).toHaveAttribute('data-state', 'coach');
    expect(screen.getByTestId('coach-seat-status')).toHaveTextContent("You're the coach of your groups");
  });

  it('a TrainingPeaks athlete account produced nothing: says so plainly and offers the team', async () => {
    getProvidersStatus.mockResolvedValue(
      served({ sciotte_trainingpeaks: { connected: true, account_role: 'athlete' } }),
    );
    requestCoachAccess.mockResolvedValue({ request: pending });
    renderSeat();

    const status = await screen.findByTestId('coach-seat-status');
    await waitFor(() => expect(status).toHaveAttribute('data-state', 'nothing'));
    expect(status).toHaveTextContent('TrainingPeaks is connected, but it gave you no coach seat.');
    await userEvent.click(screen.getByRole('button', { name: 'Ask the Dravr team' }));
    expect(requestCoachAccess).toHaveBeenCalledWith();
    await waitFor(() => expect(status).toHaveAttribute('data-state', 'requested'));
  });

  it('Intervals.icu filed the request: shows it once the server has it', async () => {
    getProvidersStatus.mockResolvedValue(served({ intervals_icu: { connected: true } }));
    getCoachAccessRequest
      .mockResolvedValueOnce({ request: null })
      .mockResolvedValue({ request: pending });
    renderSeat();

    const status = await screen.findByTestId('coach-seat-status');
    await waitFor(() => expect(status).toHaveAttribute('data-state', 'requested'));
    expect(status).toHaveTextContent('Request sent · usually within 24 h');
    expect(getCoachAccessRequest.mock.calls.length).toBeGreaterThan(1);
  });

  it('a connection that produced nothing within the window says so', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    getProvidersStatus.mockResolvedValue(served({ intervals_icu: { connected: true } }));
    renderSeat();

    const status = await screen.findByTestId('coach-seat-status');
    expect(status).toHaveAttribute('data-state', 'checking');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(COACH_SEAT_CHECK_WINDOW_MS + 100);
    });
    expect(status).toHaveAttribute('data-state', 'nothing');
    expect(status).toHaveTextContent('Intervals.icu is connected, but it gave you no coach seat.');
  });

  it('a coach who already holds the seat is stepped past without a screen', async () => {
    auth.managesRoster = true;
    const { onComplete } = renderSeat();
    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(screen.queryByText('Get your coach seat')).not.toBeInTheDocument(); // humanCoach.seatHeading
  });
});
