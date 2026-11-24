// ABOUTME: Unit tests for the messaging channels management screen
// ABOUTME: Onboarding could link a channel; nothing could list or unlink one afterwards

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert, Linking } from 'react-native';
import * as WebBrowser from 'expo-web-browser';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

const mockListLinks = jest.fn();
const mockGetAvailableChannels = jest.fn();
const mockDeleteLink = jest.fn();
const mockInitLink = jest.fn();
const mockCompleteLink = jest.fn();
jest.mock('../src/services/api', () => ({
  messagingApi: {
    listLinks: () => mockListLinks(),
    getAvailableChannels: () => mockGetAvailableChannels(),
    deleteLink: (channel: string) => mockDeleteLink(channel),
    initLink: (channel: string, appReturnUri?: string) => mockInitLink(channel, appReturnUri),
  },
  channelLinkApi: {
    completeLink: (channel: string, state: string, code: string) =>
      mockCompleteLink(channel, state, code),
  },
}));
jest.mock('expo-linking', () => ({
  createURL: jest.fn((path: string) => `dravr://${path}`),
}));

const RETURN_URI = 'dravr://messaging/link-callback';
const TELEGRAM_LINK = {
  channel: 'telegram',
  method: 'deep_link',
  code: 'pair-42',
  linking_url: 'https://t.me/DravrBot?start=pair-42',
  expires_at: '2026-09-02T12:00:00Z',
  qr_svg: null,
};

import { MessagingChannelsScreen } from '../src/screens/settings/MessagingChannelsScreen';
import { apiRefusal } from '../integration/app/helpers/apiRefusal';

/** The screen reads its two lists through React Query, so each render gets a cache of its own. */
function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MessagingChannelsScreen />
    </QueryClientProvider>,
  );
}

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
      linking_url: 'https://slack.com/openid/connect/authorize?state=pair-42',
      expires_at: '2026-09-02T12:00:00Z',
      qr_svg: null,
    });
    mockCompleteLink.mockResolvedValue({ status: 'linked', channel: 'slack', channel_user_id: 'U1' });
  });

  it('lists a linked channel with its handle', async () => {
    const { getByTestId, getByText } = renderScreen();
    await waitFor(() => expect(getByTestId('messaging-link-telegram')).toBeTruthy());
    expect(getByText('@athlete')).toBeTruthy();
  });

  it('offers only unlinked channels under Available', async () => {
    const { getByTestId, queryByTestId } = renderScreen();
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

    const { getByTestId } = renderScreen();
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
    const { getByTestId } = renderScreen();
    await waitFor(() => expect(getByTestId('messaging-error')).toBeTruthy());
    expect(getByTestId('messaging-retry')).toBeTruthy();
  });

  it('says so when every channel is already linked', async () => {
    mockListLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: null, linked_at: 'now' },
      { channel: 'slack', channel_user_id: 'sl-1', display_name: null, linked_at: 'now' },
    ]);
    const { getByTestId, queryByTestId } = renderScreen();
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
    const nothing = renderScreen();
    await waitFor(() => expect(nothing.getByTestId('messaging-none-configured')).toBeTruthy());
    expect(nothing.queryByTestId('messaging-all-linked')).toBeNull();
    // "Link one below" points at nothing here, so the linked panel says
    // something else too.
    expect(nothing.getByText(i18n.t('app.noChatAppsAvailableYet'))).toBeTruthy();
    expect(nothing.queryByText(i18n.t('app.noChatAppsLinked'))).toBeNull();
    nothing.unmount();

    mockGetAvailableChannels.mockResolvedValue(CHANNELS);
    mockListLinks.mockResolvedValue([]);
    const some = renderScreen();
    await waitFor(() => expect(some.getByTestId('messaging-link-add-telegram')).toBeTruthy());
    expect(some.queryByTestId('messaging-none-configured')).toBeNull();
    expect(some.queryByTestId('messaging-all-linked')).toBeNull();
    expect(some.getByText(i18n.t('app.noChatAppsLinked'))).toBeTruthy();
    some.unmount();

    mockListLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: null, linked_at: 'now' },
      { channel: 'slack', channel_user_id: 'sl-1', display_name: null, linked_at: 'now' },
    ]);
    const all = renderScreen();
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
    mockInitLink.mockResolvedValue(TELEGRAM_LINK);
    const { getByTestId } = renderScreen();

    await waitFor(() => expect(getByTestId('messaging-link-add-telegram')).toBeTruthy());
    fireEvent.press(getByTestId('messaging-link-add-telegram'));

    await waitFor(() => expect(mockInitLink).toHaveBeenCalledWith('telegram', RETURN_URI));
    await waitFor(() => expect(openURL).toHaveBeenCalledWith(TELEGRAM_LINK.linking_url));
    // Nowhere near the onboarding stack: the settings screen stays put and the
    // focus effect picks the link up when the athlete comes back.
    expect(mockPush).not.toHaveBeenCalled();
    openURL.mockRestore();
  });

  // carnet#1580: Slack and Discord authorize in an auth session, and the app
  // finishes the link with its own login; the external browser never holds it.
  it('authorizes an OAuth channel in an auth session and finishes the link itself', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    (WebBrowser.openAuthSessionAsync as jest.Mock).mockResolvedValueOnce({
      type: 'success',
      url: `${RETURN_URI}?channel=slack&state=pair-42&code=provider-code`,
    });
    mockListLinks.mockResolvedValue([]);
    const { getByTestId } = renderScreen();

    await waitFor(() => expect(getByTestId('messaging-link-add-slack')).toBeTruthy());
    fireEvent.press(getByTestId('messaging-link-add-slack'));

    await waitFor(() =>
      expect(mockCompleteLink).toHaveBeenCalledWith('slack', 'pair-42', 'provider-code'),
    );
    expect(mockInitLink).toHaveBeenCalledWith('slack', RETURN_URI);
    expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledWith(
      'https://slack.com/openid/connect/authorize?state=pair-42',
      RETURN_URI,
    );
    expect(openURL).not.toHaveBeenCalled();
    // The list reloads once the link is finished.
    await waitFor(() => expect(mockListLinks.mock.calls.length).toBeGreaterThan(1));
    openURL.mockRestore();
  });

  it('names the channel when the link cannot be started, instead of failing silently', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockListLinks.mockResolvedValue([]);
    mockInitLink.mockRejectedValue(apiRefusal(400, { message: 'channel not configured' }));

    const { getByTestId } = renderScreen();
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

  // carnet#803: the link started, but the device has nothing to open it with.
  // That is not the channel failing to start, so it reads as the link alert.
  it('says the link could not be opened when the device has no handler for it', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockRejectedValue(new Error('no handler'));
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockListLinks.mockResolvedValue([]);
    mockInitLink.mockResolvedValue(TELEGRAM_LINK);
    const url = TELEGRAM_LINK.linking_url;

    const { getByTestId } = renderScreen();
    await waitFor(() => expect(getByTestId('messaging-link-add-telegram')).toBeTruthy());
    fireEvent.press(getByTestId('messaging-link-add-telegram'));

    await waitFor(() =>
      expect(alertSpy).toHaveBeenCalledWith(
        i18n.t('app.couldNotOpenLink'),
        i18n.t('app.openInBrowserInstead', { url }),
      ),
    );
    expect(alertSpy).toHaveBeenCalledTimes(1);
    alertSpy.mockRestore();
    openURL.mockRestore();
  });
});
