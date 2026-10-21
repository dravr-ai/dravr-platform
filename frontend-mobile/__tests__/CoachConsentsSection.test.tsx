// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the member's "what your human coach can do" block on the phone — calendar and wellness consents on the relationship
// ABOUTME: One switch at a time, a wellness revoke asks first, refusals worded from their reason, the Intervals.icu Coach step

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachConsentsResponse } from '@pierre/shared-types';

const mockGet = jest.fn();
const mockUpdate = jest.fn();
jest.mock('../src/services/api', () => ({
  groupsApi: {
    getCoachConsents: (...args: unknown[]) => mockGet(...args),
    updateCoachConsents: (...args: unknown[]) => mockUpdate(...args),
  },
}));

import { CoachConsentsSection } from '../src/screens/groups/CoachConsentsSection';
import { useCoachConsents } from '../src/hooks/useGroups';

const GROUP_ID = 'group-1';

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

function renderBlock() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Harness />
    </QueryClientProvider>,
  );
}

function refusal(reason: string) {
  return Object.assign(new Error('Request failed'), {
    response: { status: 403, data: { message: 'English for an API caller', details: { reason } } },
  });
}

describe('CoachConsentsSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => jest.restoreAllMocks());

  it('offers the calendar consent with the Intervals.icu Coach step under it', async () => {
    mockGet.mockResolvedValue(consents({ wellness_read_supported: false }));
    const { findByTestId, getByTestId, queryByTestId } = renderBlock();

    expect(await findByTestId('coach-consent-calendar_write')).toHaveProp('value', false);
    expect(getByTestId('coach-consent-row-calendar_write')).toHaveTextContent(
      /On Intervals\.icu, Casey Coach must also be your human coach there: open intervals\.icu\/athletes, click Casey Coach and pick the Coach role, not Follower\./,
    );
    // Wellness syncs only through a link: not offered, not shown.
    expect(queryByTestId('coach-consent-wellness_read')).toBeNull();
  });

  it('shows nothing when the relationship offers no consent', async () => {
    mockGet.mockResolvedValue(consents({ calendar_write_supported: false, wellness_read_supported: false }));
    const { queryByTestId } = renderBlock();

    await waitFor(() => expect(mockGet).toHaveBeenCalledWith(GROUP_ID));
    expect(queryByTestId('coach-consents')).toBeNull();
  });

  it('switches one consent at a time, shown as made while it is on its way', async () => {
    mockGet.mockResolvedValue(consents({ calendar_write: true, wellness_read: true }));
    let answer: (answered: CoachConsentsResponse) => void = () => undefined;
    mockUpdate.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const { findByTestId, getByTestId } = renderBlock();

    expect(await findByTestId('coach-consent-calendar_write')).toHaveProp('value', true);
    expect(getByTestId('coach-consent-wellness_read')).toHaveProp('value', true);

    await act(async () => {
      fireEvent(getByTestId('coach-consent-calendar_write'), 'valueChange', false);
    });

    expect(mockUpdate).toHaveBeenCalledWith(GROUP_ID, { calendar_write: false });
    expect(getByTestId('coach-consent-calendar_write')).toHaveProp('value', false);
    expect(getByTestId('coach-consent-calendar_write')).toHaveProp('disabled', true);
    expect(getByTestId('coach-consent-wellness_read')).toHaveProp('value', true);

    await act(async () => {
      answer(consents({ calendar_write: false, wellness_read: true }));
    });
    await waitFor(() => expect(getByTestId('coach-consent-calendar_write')).toHaveProp('disabled', false));
    expect(getByTestId('coach-consent-calendar_write')).toHaveProp('value', false);
  });

  it('asks before switching wellness off, and switches nothing until the member says so', async () => {
    mockGet.mockResolvedValue(consents({ wellness_read: true }));
    mockUpdate.mockResolvedValue(consents({ wellness_read: false }));
    const { findByTestId, getByTestId } = renderBlock();

    await findByTestId('coach-consent-wellness_read');
    await act(async () => {
      fireEvent(getByTestId('coach-consent-wellness_read'), 'valueChange', false);
    });
    const ask = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; style?: string; onPress?: () => void }>,
    ];
    expect(ask[0]).toBe('Stop sharing wellness?');
    expect(ask[1]).toBe(
      "Dravr stops reading your Intervals.icu wellness through Casey Coach's account and removes what it already read.",
    );
    expect(ask[2].map((button) => [button.text, button.style])).toEqual([
      ['Cancel', 'cancel'],
      ['Stop sharing', 'destructive'],
    ]);
    expect(mockUpdate).not.toHaveBeenCalled();
    expect(getByTestId('coach-consent-wellness_read')).toHaveProp('value', true);

    await act(async () => {
      await ask[2].find((button) => button.text === 'Stop sharing')?.onPress?.();
    });
    expect(mockUpdate).toHaveBeenCalledWith(GROUP_ID, { wellness_read: false });
  });

  it('switches calendar writes on without asking', async () => {
    mockGet.mockResolvedValue(consents());
    mockUpdate.mockResolvedValue(consents({ calendar_write: true }));
    const { findByTestId, getByTestId } = renderBlock();

    await findByTestId('coach-consent-calendar_write');
    await act(async () => {
      fireEvent(getByTestId('coach-consent-calendar_write'), 'valueChange', true);
    });

    expect(mockUpdate).toHaveBeenCalledWith(GROUP_ID, { calendar_write: true });
    expect(Alert.alert).not.toHaveBeenCalled();
  });

  it('notes under the wellness switch when a source of the member own already gives Dravr it', async () => {
    mockGet.mockResolvedValue(consents({ wellness_read: true, wellness_read_skipped: 'own_wellness_source' }));
    const { findByTestId, getByTestId } = renderBlock();

    // The switch stays, so the member can still revoke what they granted.
    expect(await findByTestId('coach-consent-wellness_read')).toHaveProp('value', true);
    expect(getByTestId('coach-consent-row-wellness_read')).toHaveTextContent(
      /Your own Garmin or WHOOP connection already gives Dravr this, so the link reads none of it while that connection stays\./,
    );
    expect(getByTestId('coach-consent-row-calendar_write')).not.toHaveTextContent(/Garmin or WHOOP/);
  });

  it('words a refused consent change from its reason', async () => {
    mockGet.mockResolvedValue(consents());
    mockUpdate.mockRejectedValue(refusal('consent_grant_needs_app'));
    const { findByTestId, getByTestId } = renderBlock();

    await findByTestId('coach-consent-wellness_read');
    await act(async () => {
      fireEvent(getByTestId('coach-consent-wellness_read'), 'valueChange', true);
    });

    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Error', 'That did not go through. Try again.'));
    // Refused, the switch reads as the consent stands on the server again.
    await waitFor(() => expect(getByTestId('coach-consent-wellness_read')).toHaveProp('value', false));
  });
});
