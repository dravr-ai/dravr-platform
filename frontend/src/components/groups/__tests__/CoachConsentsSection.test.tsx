// ABOUTME: Tests for the member's "what your human coach can do" block — the calendar and wellness consents on the coach relationship
// ABOUTME: One change at a time, a wellness revoke asks first, refusals worded from their reason, the Intervals.icu Coach step
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachConsentsSection from '../CoachConsentsSection';
import { useCoachConsents } from '../../../hooks/useGroups';
import { ToastProvider } from '../../ui';
import type { CoachConsentsResponse } from '@pierre/shared-types';

const GROUP_ID = 'group-1';

vi.mock('../../../services/api', () => ({
  groupsApi: {
    getCoachConsents: vi.fn(),
    updateCoachConsents: vi.fn(),
  },
}));

const { groupsApi } = await import('../../../services/api');

function consents(overrides: Partial<CoachConsentsResponse> = {}): CoachConsentsResponse {
  return {
    calendar_write: false,
    calendar_write_at: null,
    calendar_write_supported: true,
    wellness_read: false,
    wellness_read_at: null,
    wellness_read_supported: true,
    wellness_read_skipped: null,
    ...overrides,
  };
}

/** The block as Group info renders it: fed by the member's coach-consents read. */
function Harness() {
  const { consents: held } = useCoachConsents(GROUP_ID, true);
  return held ? <CoachConsentsSection groupId={GROUP_ID} consents={held} coach="Casey Coach" /> : null;
}

function refusal(status: number, reason: string) {
  return Object.assign(new Error(`Request failed with status code ${status}`), {
    response: { status, data: { code: 'PermissionDenied', message: 'English for an API caller', details: { reason } } },
  });
}

function renderBlock() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Harness />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('CoachConsentsSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('offers the calendar consent with the Intervals.icu Coach step beside it', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(consents({ wellness_read_supported: false }));
    renderBlock();

    const block = await screen.findByTestId('coach-consents');
    expect(
      within(block).getByRole('checkbox', { name: /Let Casey Coach put workouts on your Intervals\.icu calendar/ }),
    ).not.toBeChecked();
    expect(within(block).getByTestId('coach-consent-calendar-step')).toHaveTextContent(
      'On Intervals.icu, Casey Coach must also be your human coach there: open intervals.icu/athletes, click Casey Coach and pick the Coach role, not Follower.',
    );
    // Wellness syncs only through a link: not offered, not shown.
    expect(within(block).queryByTestId('coach-consent-wellness_read')).toBeNull();
  });

  it('shows nothing when the relationship offers no consent', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(
      consents({ calendar_write_supported: false, wellness_read_supported: false }),
    );
    renderBlock();

    await waitFor(() => expect(groupsApi.getCoachConsents).toHaveBeenCalledWith(GROUP_ID));
    expect(screen.queryByTestId('coach-consents')).toBeNull();
  });

  it('changes one consent at a time, showing it as made while it is on its way', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(consents({ calendar_write: true, wellness_read: true }));
    let answer: (answered: CoachConsentsResponse) => void = () => undefined;
    vi.mocked(groupsApi.updateCoachConsents).mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const user = userEvent.setup();
    renderBlock();

    const calendar = await screen.findByTestId('coach-consent-calendar_write');
    const wellness = screen.getByTestId('coach-consent-wellness_read');
    expect(calendar).toBeChecked();
    expect(wellness).toBeChecked();

    await user.click(calendar);

    await waitFor(() =>
      expect(groupsApi.updateCoachConsents).toHaveBeenCalledWith(GROUP_ID, { calendar_write: false }),
    );
    expect(calendar).not.toBeChecked();
    expect(calendar).toBeDisabled();
    expect(wellness).toBeChecked();

    answer(consents({ calendar_write: false, wellness_read: true }));
    expect(await screen.findByText('Sharing updated')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('coach-consent-calendar_write')).not.toBeChecked());
  });

  it('asks before turning wellness off, and changes nothing when the member keeps sharing', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(consents({ wellness_read: true }));
    vi.mocked(groupsApi.updateCoachConsents).mockResolvedValue(consents({ wellness_read: false }));
    const user = userEvent.setup();
    renderBlock();

    const wellness = await screen.findByTestId('coach-consent-wellness_read');
    await user.click(wellness);
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Stop sharing wellness?');
    expect(dialog).toHaveTextContent(
      "Dravr stops reading your Intervals.icu wellness through Casey Coach's account and removes what it already read.",
    );
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(groupsApi.updateCoachConsents).not.toHaveBeenCalled();
    expect(wellness).toBeChecked();

    await user.click(wellness);
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Stop sharing' }));
    await waitFor(() => expect(groupsApi.updateCoachConsents).toHaveBeenCalledWith(GROUP_ID, { wellness_read: false }));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('turns calendar writes on without asking', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(consents());
    vi.mocked(groupsApi.updateCoachConsents).mockResolvedValue(consents({ calendar_write: true }));
    const user = userEvent.setup();
    renderBlock();

    await user.click(await screen.findByTestId('coach-consent-calendar_write'));

    await waitFor(() => expect(groupsApi.updateCoachConsents).toHaveBeenCalledWith(GROUP_ID, { calendar_write: true }));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('notes under the wellness control when a source of the member own already gives Dravr it', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(
      consents({ wellness_read: true, wellness_read_skipped: 'own_wellness_source' }),
    );
    renderBlock();

    // The control stays, so the member can still revoke what they granted.
    expect(await screen.findByTestId('coach-consent-wellness_read')).toBeChecked();
    expect(screen.getByTestId('coach-consent-note-wellness_read')).toHaveTextContent(
      'Your own Garmin or WHOOP connection already gives Dravr this, so the link reads none of it while that connection stays.',
    );
    expect(screen.queryByTestId('coach-consent-note-calendar_write')).toBeNull();
  });

  it('words a refused consent change from its reason, not the server message', async () => {
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(consents());
    vi.mocked(groupsApi.updateCoachConsents).mockRejectedValue(refusal(403, 'consent_grant_needs_app'));
    const user = userEvent.setup();
    renderBlock();

    await user.click(await screen.findByTestId('coach-consent-calendar_write'));

    expect(await screen.findByText('That did not go through. Try again.')).toBeInTheDocument();
    expect(screen.queryByText('English for an API caller')).toBeNull();
    // Refused, the consent reads as it stands on the server again.
    await waitFor(() => expect(screen.getByTestId('coach-consent-calendar_write')).not.toBeChecked());
  });
});
