// ABOUTME: Unit tests for the messaging channels management screen
// ABOUTME: Onboarding could link a channel; nothing could list or unlink one afterwards

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

const mockListLinks = jest.fn();
const mockGetAvailableChannels = jest.fn();
const mockDeleteLink = jest.fn();
jest.mock('../src/services/api', () => ({
  messagingApi: {
    listLinks: () => mockListLinks(),
    getAvailableChannels: () => mockGetAvailableChannels(),
    deleteLink: (channel: string) => mockDeleteLink(channel),
  },
}));

import { MessagingChannelsScreen } from '../src/screens/settings/MessagingChannelsScreen';

const CHANNELS = [
  { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
  { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
];

describe('MessagingChannelsScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetAvailableChannels.mockResolvedValue(CHANNELS);
    mockListLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: '@athlete', linked_at: 'now' },
    ]);
    mockDeleteLink.mockResolvedValue(undefined);
  });

  it('lists a linked channel with its handle', async () => {
    const { getByTestId, getByText } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-link-telegram')).toBeTruthy());
    expect(getByText('@athlete')).toBeTruthy();
  });

  it('offers only unlinked channels under Available', async () => {
    const { getByTestId, queryByTestId } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-link-add-slack')).toBeTruthy());
    // Telegram is already linked — offering to link it again would 400.
    expect(queryByTestId('messaging-link-add-telegram')).toBeNull();
  });

  it('unlinks a channel and refreshes the list', async () => {
    const alertSpy = jest
      .spyOn(Alert, 'alert')
      .mockImplementation((_t, _m, buttons) => {
        // Confirm the destructive action the way a user tapping "Unlink" would.
        const confirm = (buttons ?? []).find((b) => b.text === 'Unlink');
        confirm?.onPress?.();
      });

    const { getByTestId } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-unlink-telegram')).toBeTruthy());
    fireEvent.press(getByTestId('messaging-unlink-telegram'));

    await waitFor(() => expect(mockDeleteLink).toHaveBeenCalledWith('telegram'));
    // Refetched, so the screen reflects the server rather than local guesswork.
    await waitFor(() => expect(mockListLinks).toHaveBeenCalledTimes(2));
    alertSpy.mockRestore();
  });

  it('surfaces a load failure instead of showing an empty list', async () => {
    // An empty list here reads as "nothing linked" and would invite someone to
    // re-link a channel they already have.
    mockListLinks.mockRejectedValueOnce(new Error('offline'));
    const { getByTestId } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-error')).toBeTruthy());
    expect(getByTestId('messaging-retry')).toBeTruthy();
  });

  it('says so when every channel is already linked', async () => {
    mockListLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: null, linked_at: 'now' },
      { channel: 'slack', channel_user_id: 'sl-1', display_name: null, linked_at: 'now' },
    ]);
    const { getByTestId } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-all-linked')).toBeTruthy());
  });
});
