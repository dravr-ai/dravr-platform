// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile group chat-app card and its sheet — who meets the card, what each answer does
// ABOUTME: Only the owning coach, once across devices; Configure opens the sheet on that app, a landed link closes it (carnet#1525)

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachingGroup } from '@pierre/shared-types';
import { GroupChatAppOffer, useGroupChatAppOffer } from '../GroupChatAppOffer';
import { ChatAppSheet } from '../ChatAppSheet';
import { messagingApi, userApi } from '../../../services/api';

jest.mock('../../../contexts/AuthContext', () => ({ useAuth: () => ({ user: { id: 'coach-1' } }) }));
jest.mock('../../../services/api', () => ({
  userApi: { getOnboardingStatus: jest.fn(), setOnboardingStep: jest.fn() },
  messagingApi: { getAvailableChannels: jest.fn(), listLinks: jest.fn(), initLink: jest.fn() },
}));
jest.mock('expo-device', () => ({
  deviceType: 1, // Device.DeviceType.PHONE
  DeviceType: { UNKNOWN: 0, PHONE: 1, TABLET: 2, DESKTOP: 3, TV: 4 },
}));
jest.mock('react-native-webview', () => {
  const ReactMock = require('react');
  const { View } = require('react-native');
  return { WebView: (props: { testID?: string }) => ReactMock.createElement(View, { testID: props.testID }) };
});

const getOnboardingStatus = userApi.getOnboardingStatus as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;
const getAvailableChannels = messagingApi.getAvailableChannels as jest.Mock;
const listLinks = messagingApi.listLinks as jest.Mock;
const initLink = messagingApi.initLink as jest.Mock;

const TELEGRAM = { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true };
const WHATSAPP = { channel: 'whatsapp', display_name: 'WhatsApp', method: 'deep_link', recommended: false };

const GROUP = {
  id: 'g-1',
  tenant_id: 't-1',
  name: 'Léa · Marathon de Montréal',
  description: null,
  agent_id: 'agent-endurance',
  agent_title: 'Endurance',
  agent_handle: 'endurance',
  owner_id: 'coach-1',
  coach_user_id: 'coach-1',
  coach_display_name: 'Philippe',
  peer_data_sharing: false,
  respond_mode: 'all',
  digest_mode: 'off',
  max_members: 10,
  is_active: true,
  created_at: '2026-11-12T09:00:00Z',
  updated_at: '2026-11-12T09:00:00Z',
} as CoachingGroup;

function status(steps: { step_id: string; status: string }[] = []) {
  return {
    needs_provider_connection: false,
    pillars_covered: 0,
    pillars_total: 7,
    onboarding_complete: false,
    steps,
    chosen_channel: null,
    coaches_others: true,
  };
}

function Harness({ group }: { group: CoachingGroup | null }) {
  const offer = useGroupChatAppOffer(group);
  const [sheet, setSheet] = React.useState<string | null>(null);
  return (
    <>
      {offer ? <GroupChatAppOffer offer={offer} agentLabel="Endurance" onConfigure={setSheet} /> : null}
      {sheet ? <ChatAppSheet visible initialChannel={sheet} onClose={() => setSheet(null)} /> : null}
    </>
  );
}

function renderOffer(group: CoachingGroup | null = GROUP) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  render(
    <QueryClientProvider client={client}>
      <Harness group={group} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  getOnboardingStatus.mockResolvedValue(status());
  getAvailableChannels.mockResolvedValue([TELEGRAM, WHATSAPP]);
  listLinks.mockResolvedValue([]);
  setOnboardingStep.mockResolvedValue(undefined);
  initLink.mockImplementation(async (channel: string) => ({
    channel,
    method: 'deep_link',
    code: 'abc',
    linking_url: `https://example.test/${channel}?start=abc`,
    expires_at: '2099-01-01T00:00:00Z',
    qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
  }));
});

describe('GroupChatAppOffer (mobile)', () => {
  it('offers the recommended app to the coach who owns the group, and Configure opens its link', async () => {
    renderOffer();
    expect(await screen.findByText('Write to me from Telegram too?')).toBeTruthy();
    fireEvent.press(screen.getByTestId('group-chat-app-offer-configure'));
    expect(await screen.findByText('Open Telegram')).toBeTruthy();
    expect(screen.getByTestId('chat-app-use-another')).toBeTruthy();
    expect(setOnboardingStep).not.toHaveBeenCalled();
  });

  it('stores « Not now » on the onboarding record and hides the card', async () => {
    setOnboardingStep.mockImplementation(async () => {
      getOnboardingStatus.mockResolvedValue(status([{ step_id: 'messaging_configure', status: 'skipped' }]));
    });
    renderOffer();
    fireEvent.press(await screen.findByTestId('group-chat-app-offer-not-now'));
    // The answer is stored first, then the card goes; a refused write would
    // keep it up with an alert, which must not be what happened here. The
    // card leaves on a cache write plus a re-render, which a loaded CI runner
    // under coverage has been seen to take past the default second. The test's
    // own budget sits above that wait, so a miss is reported by the wait rather
    // than cut short by Jest's five-second default.
    await waitFor(() => expect(setOnboardingStep).toHaveBeenCalledWith('messaging_configure', 'skipped'));
    expect(screen.queryByRole('alert')).toBeNull();
    await waitFor(() => expect(screen.queryByTestId('group-chat-app-offer')).toBeNull(), { timeout: 5_000 });
  }, 15_000);

  it('keeps the card with the error when « Not now » was not stored', async () => {
    setOnboardingStep.mockRejectedValue(new Error('offline'));
    renderOffer();
    fireEvent.press(await screen.findByTestId('group-chat-app-offer-not-now'));
    expect(await screen.findByText('Your choice wasn’t saved. Try again.')).toBeTruthy();
    expect(screen.getByTestId('group-chat-app-offer')).toBeTruthy();
  });

  it('is never shown once answered on any device, nor to a coach already linked', async () => {
    getOnboardingStatus.mockResolvedValue(status([{ step_id: 'messaging_configure', status: 'skipped' }]));
    renderOffer();
    await waitFor(() => expect(getOnboardingStatus).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
    expect(getAvailableChannels).not.toHaveBeenCalled();
  });

  it('is not shown to a coach who already linked an app', async () => {
    listLinks.mockResolvedValue([
      { channel: 'whatsapp', channel_user_id: 'wa-1', display_name: null, linked_at: '2026-11-01T10:00:00Z' },
    ]);
    renderOffer();
    await waitFor(() => expect(listLinks).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
  });

  it('is not shown in someone else’s group', async () => {
    renderOffer({ ...GROUP, owner_id: 'someone-else' });
    await waitFor(() => expect(getOnboardingStatus).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
  });

  // The link lands on the next poll (CHANNEL_LINK_POLL_INTERVAL_MS), so the
  // test waits past one interval, with room for a loaded runner.
  it('closes the sheet once the link lands, and records the offer complete', async () => {
    renderOffer();
    fireEvent.press(await screen.findByTestId('group-chat-app-offer-configure'));
    await screen.findByText('Open Telegram');
    listLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: null, linked_at: '2026-11-12T10:00:00Z' },
    ]);
    await waitFor(() => expect(setOnboardingStep).toHaveBeenCalledWith('messaging_configure', 'complete'), {
      timeout: 15_000,
    });
    await waitFor(() => expect(screen.queryByTestId('chat-app-sheet')).toBeNull());
  }, 30_000);
});
