// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile ChatAppConnect — the chat-app sheet's deep-link link view + poll-to-complete
// ABOUTME: Verifies a phone taps straight through (no QR), a tablet gets the QR handoff, the poll reports, and Use another app

import React from 'react';
import { Alert, Linking } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ChatAppConnect } from '../ChatAppConnect';
import { messagingApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  messagingApi: { initLink: jest.fn(), listLinks: jest.fn() },
}));
// The QR is a device-to-device handoff, so which device this is decides whether
// it renders at all. Backed by a mutable so a test can be a tablet.
let mockDeviceType = 1; // Device.DeviceType.PHONE
jest.mock('expo-device', () => ({
  get deviceType() {
    return mockDeviceType;
  },
  DeviceType: { UNKNOWN: 0, PHONE: 1, TABLET: 2, DESKTOP: 3, TV: 4 },
}));
jest.mock('react-native-webview', () => {
  const ReactMock = require('react');
  const { View } = require('react-native');
  return { WebView: (props: { testID?: string }) => ReactMock.createElement(View, { testID: props.testID }) };
});

const onLinked = jest.fn();
const onUseAnother = jest.fn();
const initLink = messagingApi.initLink as jest.Mock;
const listLinks = messagingApi.listLinks as jest.Mock;

function renderScreen({ withOthers = false }: { withOthers?: boolean } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  render(
    <QueryClientProvider client={client}>
      <ChatAppConnect
        channel="telegram"
        displayName="Telegram"
        onLinked={onLinked}
        onUseAnother={withOthers ? onUseAnother : undefined}
      />
    </QueryClientProvider>,
  );
}

describe('ChatAppConnect', () => {
  beforeEach(() => {
    mockDeviceType = 1; // PHONE — the common case; a test opts into TABLET
    onLinked.mockClear();
    onUseAnother.mockClear();
    initLink.mockReset().mockResolvedValue({
      channel: 'telegram',
      method: 'deep_link',
      code: 'abc',
      linking_url: 'https://t.me/DravrBot?start=abc',
      expires_at: '2030-01-01T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    });
    listLinks.mockReset().mockResolvedValue([]);
  });

  it('leads a phone straight to the open button, with no QR to scan', async () => {
    renderScreen();
    // The button is the whole flow on a phone: the deep link carries the pairing
    // code, so one tap opens Telegram with it already in hand.
    expect(await screen.findByText('Open Telegram')).toBeTruthy();
    // A QR here would ask the athlete to scan the screen they are holding.
    expect(screen.queryByTestId('messaging-qr')).toBeNull();
  });

  it('offers Use another app only when the workspace has another app', async () => {
    renderScreen({ withOthers: true });
    fireEvent.press(await screen.findByTestId('chat-app-use-another'));
    expect(onUseAnother).toHaveBeenCalledTimes(1);
  });

  it('leaves Use another app out when this is the only app', async () => {
    renderScreen();
    await screen.findByText('Open Telegram');
    expect(screen.queryByTestId('chat-app-use-another')).toBeNull();
  });

  describe('the open button (carnet#803)', () => {
    const LINK = 'https://t.me/DravrBot?start=abc';

    afterEach(() => {
      jest.restoreAllMocks();
    });

    it('opens the chat app on the pairing link', async () => {
      const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
      renderScreen();
      fireEvent.press(await screen.findByText('Open Telegram'));
      await waitFor(() => expect(openURL).toHaveBeenCalledWith(LINK));
    });

    it('says so when the device cannot open the link, instead of doing nothing', async () => {
      jest.spyOn(Linking, 'openURL').mockRejectedValue(new Error('no handler'));
      const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
      renderScreen();
      fireEvent.press(await screen.findByText('Open Telegram'));
      await waitFor(() =>
        expect(alert).toHaveBeenCalledWith(
          i18n.t('app.couldNotOpenLink'),
          i18n.t('app.openInBrowserInstead', { url: LINK }),
        ),
      );
    });
  });

  it('offers the QR handoff on a tablet, where the chat app may live elsewhere', async () => {
    mockDeviceType = 2; // TABLET
    renderScreen();
    expect(await screen.findByTestId('messaging-qr')).toBeTruthy();
    expect(screen.getByText('Open Telegram')).toBeTruthy();
  });

  it('reports via onLinked once the channel link appears', async () => {
    listLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'u1', display_name: null, linked_at: 'now' },
    ]);
    renderScreen();
    await waitFor(() => expect(onLinked).toHaveBeenCalled());
  });
});
