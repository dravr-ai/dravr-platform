// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the TrainingPeaks link section on the phone — the coach's picker, the propose payload, each refusal's words
// ABOUTME: And the member's unlink behind a confirm, never the server's English refusal text

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { DelegatedConnection, GroupMember } from '@pierre/shared-types';
import { DELEGATION_REFUSAL_KEY } from '@pierre/shared-constants';
import { i18n } from '@pierre/i18n';

const mockPropose = jest.fn();
const mockEnd = jest.fn();
const mockRoster = jest.fn();
jest.mock('../src/services/api', () => ({
  oauthApi: { getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }) },
  groupsApi: {
    getDelegationRoster: (...args: unknown[]) => mockRoster(...args),
    proposeDelegatedConnection: (...args: unknown[]) => mockPropose(...args),
    confirmDelegatedConnection: jest.fn(),
    endDelegatedConnection: (...args: unknown[]) => mockEnd(...args),
  },
}));

import { DelegatedConnectionsSection } from '../src/screens/groups/DelegatedConnectionsSection';

const GROUP_ID = 'group-1';

function member(userId: string, name: string): GroupMember {
  return {
    id: `membership-${userId}`,
    group_id: GROUP_ID,
    user_id: userId,
    role: 'member',
    peer_sharing_consent: true,
    consent_given_at: '2026-05-01T08:00:00Z',
    joined_at: '2026-05-01T08:00:00Z',
    display_name: name,
  };
}

function link(overrides: Partial<DelegatedConnection> = {}): DelegatedConnection {
  return {
    id: 'dc-1',
    group_id: GROUP_ID,
    provider: 'trainingpeaks',
    coach_user_id: 'user-coach',
    coach_display_name: 'Casey Coach',
    member_user_id: 'user-alex',
    member_display_name: 'Alex',
    provider_athlete_id: '900001',
    provider_athlete_name: 'Alex Athlete',
    status: 'proposed',
    proposed_at: '2026-09-24T08:00:00Z',
    confirmed_at: null,
    ...overrides,
  };
}

/** An axios-shaped refusal carrying `details.reason`. */
function refusal(reason: string) {
  return Object.assign(new Error('Request failed'), {
    response: { status: 400, data: { message: 'English for an API caller', details: { reason } } },
  });
}

const MEMBERS = [member('user-alex', 'Alex'), member('user-blair', 'Blair')];

function renderSection(props: Partial<React.ComponentProps<typeof DelegatedConnectionsSection>> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <DelegatedConnectionsSection groupId={GROUP_ID} mode="coach" connections={[]} members={MEMBERS} {...props} />
    </QueryClientProvider>,
  );
}

describe('DelegatedConnectionsSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockRoster.mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        { provider_athlete_id: '900001', display_name: 'Alex Athlete', connection: null, suggested_member_user_id: 'user-alex' },
        {
          provider_athlete_id: '900002',
          display_name: 'Blair Biker',
          connection: link({ id: 'dc-2', member_user_id: 'user-blair', member_display_name: 'Blair', provider_athlete_id: '900002' }),
          suggested_member_user_id: null,
        },
      ],
    });
  });

  afterEach(() => jest.restoreAllMocks());

  it('offers only unlinked members in the picker and proposes the one picked', async () => {
    mockPropose.mockResolvedValue(link());
    const { findByTestId, getByTestId, queryByTestId } = renderSection({
      connections: [link({ id: 'dc-2', member_user_id: 'user-blair' })],
    });

    expect(await findByTestId('delegation-roster-row-900002')).toHaveTextContent(/Waiting for Blair to confirm/);
    fireEvent.press(getByTestId('delegation-propose-900001'));

    expect(getByTestId('delegation-member-option-user-alex')).toHaveTextContent('Alex (suggested)');
    // Blair already holds a live link, so the picker does not offer them again.
    expect(queryByTestId('delegation-member-option-user-blair')).toBeNull();

    await act(async () => {
      fireEvent.press(getByTestId('delegation-member-option-user-alex'));
    });
    expect(mockPropose).toHaveBeenCalledWith(GROUP_ID, {
      provider: 'trainingpeaks',
      provider_athlete_id: '900001',
      member_user_id: 'user-alex',
    });
  });

  it('words a propose refusal from its reason', async () => {
    mockPropose.mockRejectedValue(refusal('athlete_already_linked'));
    const { findByTestId, getByTestId } = renderSection();

    fireEvent.press(await findByTestId('delegation-propose-900001'));
    await act(async () => {
      fireEvent.press(getByTestId('delegation-member-option-user-alex'));
    });

    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Error', 'This athlete is already linked.'));
  });

  it('maps every refusal reason to a sentence of its own, never the server text', async () => {
    const en = i18n.getFixedT('en');
    for (const [reason, key] of Object.entries(DELEGATION_REFUSAL_KEY)) {
      mockRoster.mockRejectedValueOnce(refusal(reason));
      const view = renderSection({ onOpenConnections: jest.fn() });
      const refused = await view.findByTestId('delegation-roster-refused');
      expect(refused).toHaveTextContent(en(key), { exact: false });
      expect(refused).not.toHaveTextContent(/English for an API caller/);
      view.unmount();
    }
  });

  it('sends a coach whose notice is outdated to their connections', async () => {
    mockRoster.mockRejectedValue(refusal('trainingpeaks_terms_outdated'));
    const onOpenConnections = jest.fn();
    const { findByTestId } = renderSection({ onOpenConnections });

    fireEvent.press(await findByTestId('delegation-open-connections'));
    expect(onOpenConnections).toHaveBeenCalledTimes(1);
  });

  it('unlinks a confirmed member link behind a confirm', async () => {
    mockEnd.mockResolvedValue(undefined);
    const { findByTestId } = renderSection({
      mode: 'member',
      connections: [link({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' })],
    });

    expect(await findByTestId('delegation-linked')).toHaveTextContent(
      /Your TrainingPeaks workouts are read through Casey Coach's account\./,
    );
    fireEvent.press(await findByTestId('delegation-unlink-dc-1'));
    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Unlink TrainingPeaks?');
    await act(async () => {
      await confirm[2].find((button) => button.text === 'Unlink')?.onPress?.();
    });
    expect(mockEnd).toHaveBeenCalledWith(GROUP_ID, 'dc-1');
  });
});
