// ABOUTME: Unit tests for the messaging channels management screen
// ABOUTME: Onboarding could link a channel; nothing could list or unlink one afterwards

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert, Linking } from 'react-native';
import { i18n } from '@pierre/i18n';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

const mockListLinks = jest.fn();
const mockGetAvailableChannels = jest.fn();
const mockDeleteLink = jest.fn();
const mockInitLink = jest.fn();
jest.mock('../src/services/api', () => ({
  messagingApi: {
    listLinks: () => mockListLinks(),
    getAvailableChannels: () => mockGetAvailableChannels(),
    deleteLink: (channel: string) => mockDeleteLink(channel),
    initLink: (channel: string) => mockInitLink(channel),
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
    mockInitLink.mockResolvedValue({
      channel: 'slack',
      method: 'oauth',
      code: null,
      linking_url: 'https://slack.com/oauth/v2/authorize?state=pair-42',
      expires_at: '2026-09-02T12:00:00Z',
      qr_svg: null,
    });
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
    const { getByTestId, queryByTestId } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-all-linked')).toBeTruthy());
    expect(queryByTestId('messaging-none-configured')).toBeNull();
  });

  // Empty-because-all-linked and empty-because-none-configured are different
  // states. The screen showed the first whenever the available list was empty,
  // so a tenant with no channel configured read "nothing is linked" and "every
  // available app is linked" on the same page, over a list with nothing in it.
  it('separates nothing-configured from everything-linked, and from unlinked-and-waiting', async () => {
    mockGetAvailableChannels.mockResolvedValue([]);
    mockListLinks.mockResolvedValue([]);
    const nothing = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(nothing.getByTestId('messaging-none-configured')).toBeTruthy());
    expect(nothing.queryByTestId('messaging-all-linked')).toBeNull();
    // "Link one below" points at nothing here, so the linked panel says
    // something else too.
    expect(nothing.getByText(i18n.t('app.noChatAppsAvailableYet'))).toBeTruthy();
    expect(nothing.queryByText(i18n.t('app.noChatAppsLinked'))).toBeNull();
    nothing.unmount();

    mockGetAvailableChannels.mockResolvedValue(CHANNELS);
    mockListLinks.mockResolvedValue([]);
    const some = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(some.getByTestId('messaging-link-add-telegram')).toBeTruthy());
    expect(some.queryByTestId('messaging-none-configured')).toBeNull();
    expect(some.queryByTestId('messaging-all-linked')).toBeNull();
    expect(some.getByText(i18n.t('app.noChatAppsLinked'))).toBeTruthy();
    some.unmount();

    mockListLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: null, linked_at: 'now' },
      { channel: 'slack', channel_user_id: 'sl-1', display_name: null, linked_at: 'now' },
    ]);
    const all = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(all.getByTestId('messaging-all-linked')).toBeTruthy());
    expect(all.queryByTestId('messaging-none-configured')).toBeNull();

    // Three states, three sentences.
    const texts = [
      i18n.t('app.noChatAppsConfigured'),
      i18n.t('app.everyChatAppLinked'),
      i18n.t('app.noChatAppsLinked'),
    ];
    expect(new Set(texts).size).toBe(3);
  });

  // Onboarding hands the athlete to the chat app by opening the provider's own
  // linking URL. Settings only listed what somebody had already linked; it now
  // takes the same path rather than a second one.
  it('opens the provider deep link when an available channel is tapped', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    mockListLinks.mockResolvedValue([]);
    const { getByTestId } = render(<MessagingChannelsScreen />);

    await waitFor(() => expect(getByTestId('messaging-link-add-slack')).toBeTruthy());
    fireEvent.press(getByTestId('messaging-link-add-slack'));

    await waitFor(() => expect(mockInitLink).toHaveBeenCalledWith('slack'));
    await waitFor(() =>
      expect(openURL).toHaveBeenCalledWith('https://slack.com/oauth/v2/authorize?state=pair-42'),
    );
    // Nowhere near the onboarding stack: the settings screen stays put and the
    // focus effect picks the link up when the athlete comes back.
    expect(mockPush).not.toHaveBeenCalled();
    openURL.mockRestore();
  });

  it('names the channel when the link cannot be started, instead of failing silently', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockListLinks.mockResolvedValue([]);
    mockInitLink.mockRejectedValue(new Error('channel not configured'));

    const { getByTestId } = render(<MessagingChannelsScreen />);
    await waitFor(() => expect(getByTestId('messaging-link-add-slack')).toBeTruthy());
    fireEvent.press(getByTestId('messaging-link-add-slack'));

    await waitFor(() =>
      expect(alertSpy).toHaveBeenCalledWith(
        i18n.t('app.couldNotStartConnection', { channel: 'Slack' }),
        'channel not configured',
      ),
    );
    expect(openURL).not.toHaveBeenCalled();
    alertSpy.mockRestore();
    openURL.mockRestore();
  });
});
