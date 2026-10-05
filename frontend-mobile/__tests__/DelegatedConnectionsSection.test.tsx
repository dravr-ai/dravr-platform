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
    read_refused: null,
    ...overrides,
  };
}

/** An axios-shaped refusal carrying `details.reason`. */
/**
 * A link step's refusal as the server sends it: the reason, and the platform
 * it is about whenever one is known (`null` for none).
 */
function refusal(reason: string, provider: string | null = 'trainingpeaks') {
  const details = provider ? { reason, provider } : { reason };
  return Object.assign(new Error('Request failed'), {
    response: { status: 400, data: { message: 'English for an API caller', details } },
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

  it('proposes on the platform the roster was read from', async () => {
    mockRoster.mockResolvedValue({
      provider: 'intervals_icu',
      athletes: [
        {
          provider_athlete_id: 'i201',
          display_name: 'Alex Athlete',
          connection: null,
          suggested_member_user_id: 'user-alex',
        },
      ],
    });
    mockPropose.mockResolvedValue(link());
    const { findByTestId, getByTestId } = renderSection();

    fireEvent.press(await findByTestId('delegation-propose-i201'));
    await act(async () => {
      fireEvent.press(getByTestId('delegation-member-option-user-alex'));
    });
    expect(mockPropose).toHaveBeenCalledWith(GROUP_ID, {
      provider: 'intervals_icu',
      provider_athlete_id: 'i201',
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
      expect(refused).toHaveTextContent(en(key, { platform: 'TrainingPeaks' }), { exact: false });
      expect(refused).not.toHaveTextContent(/English for an API caller/);
      view.unmount();
    }
  });

  it('names the platform a refusal is about: Intervals.icu for an OAuth-linked coach', async () => {
    mockRoster.mockRejectedValue(refusal('coach_platform_api_key_required', 'intervals_icu'));
    const { findByTestId } = renderSection({ onOpenConnections: jest.fn() });

    const refused = await findByTestId('delegation-roster-refused');
    expect(refused).toHaveTextContent(/Reconnect Intervals\.icu with your API key to read your roster\./);
    expect(refused).not.toHaveTextContent(/TrainingPeaks/);
  });

  it('names every platform a coach can connect while none is connected', async () => {
    mockRoster.mockRejectedValue(refusal('coach_platform_not_connected', null));
    const { findByTestId } = renderSection({ onOpenConnections: jest.fn() });

    expect(await findByTestId('delegation-roster-refused')).toHaveTextContent(
      /Connect TrainingPeaks \/ Intervals\.icu with the account your athletes are on/,
    );
  });

  it('names the platform the coach links run on when the roster read fails', async () => {
    mockRoster.mockRejectedValue(
      Object.assign(new Error('Request failed'), {
        response: { status: 500, data: { message: 'English for an API caller' } },
      }),
    );
    const { findByTestId } = renderSection({ connections: [link({ provider: 'intervals_icu' })] });

    // A transient failure names no platform; the coach's own links still do.
    expect(await findByTestId('delegation-roster-refused')).toHaveTextContent(
      'Your Intervals.icu roster could not be read. Try again in a moment.',
    );
  });

  it('sends a coach whose notice is outdated to their connections', async () => {
    mockRoster.mockRejectedValue(refusal('coach_platform_terms_outdated'));
    const onOpenConnections = jest.fn();
    const { findByTestId } = renderSection({ onOpenConnections });

    fireEvent.press(await findByTestId('delegation-open-connections'));
    expect(onOpenConnections).toHaveBeenCalledTimes(1);
  });

  it('sends a coach whose TrainingPeaks account is not theirs by email to their connections', async () => {
    mockRoster.mockRejectedValue(refusal('coach_platform_email_mismatch'));
    const onOpenConnections = jest.fn();
    const { findByTestId } = renderSection({ onOpenConnections });

    expect(await findByTestId('delegation-roster-refused')).toHaveTextContent(
      /uses a different email than your Dravr account/,
    );
    fireEvent.press(await findByTestId('delegation-open-connections'));
    expect(onOpenConnections).toHaveBeenCalledTimes(1);
  });

  it('words a propose refused because the athlete is not the member by email', async () => {
    mockPropose.mockRejectedValue(refusal('athlete_email_mismatch'));
    const { findByTestId, getByTestId } = renderSection();

    fireEvent.press(await findByTestId('delegation-propose-900001'));
    await act(async () => {
      fireEvent.press(getByTestId('delegation-member-option-user-alex'));
    });

    await waitFor(() =>
      expect(Alert.alert).toHaveBeenCalledWith(
        'Error',
        "This athlete's TrainingPeaks email is not the member's Dravr email, so they cannot be linked.",
      ),
    );
  });

  it('tells the coach why a confirmed link reads nothing, and keeps the unlink', async () => {
    mockRoster.mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        {
          provider_athlete_id: '900001',
          display_name: 'Alex Athlete',
          connection: link({
            status: 'confirmed',
            confirmed_at: '2026-09-24T09:00:00Z',
            read_refused: 'athlete_email_missing',
          }),
          suggested_member_user_id: null,
        },
      ],
    });
    const { findByTestId, getByTestId } = renderSection();

    const row = await findByTestId('delegation-roster-row-900001');
    expect(row).toHaveTextContent(
      /TrainingPeaks shares no email for this athlete, so Dravr cannot match them to a member\./,
    );
    expect(row).not.toHaveTextContent(/Linked to/);
    expect(getByTestId('delegation-unlink-dc-1')).toBeTruthy();
  });

  it('tells the member why their confirmed link reads nothing', async () => {
    const { findByTestId } = renderSection({
      mode: 'member',
      connections: [
        link({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z', read_refused: 'athlete_email_mismatch' }),
      ],
    });

    expect(await findByTestId('delegation-read-refused')).toHaveTextContent(
      "This athlete's TrainingPeaks email is not the member's Dravr email, so they cannot be linked.",
    );
    expect(await findByTestId('delegation-linked')).not.toHaveTextContent(/read through Casey Coach/);
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
