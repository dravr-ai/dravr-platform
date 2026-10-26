// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The shell reconnect banner over a mocked provider status — names each flagged connected provider once, hides when healthy or disconnected
// ABOUTME: Pins the Connections target, the alert live region, the pane where it stays away, and the re-read when the athlete leaves that pane

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  PROVIDERS_CONNECTED,
  PROVIDERS_RECONNECT,
} from '../../../integration/app/helpers/homeFixtures';

const mockPush = jest.fn();
let mockPathname = '/';
jest.mock('expo-router', () =>
  require('../../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({
      push: mockPush,
      replace: jest.fn(),
      back: jest.fn(),
      navigate: jest.fn(),
      canGoBack: () => true,
    }),
    usePathname: () => mockPathname,
  }),
);

const mockGetProvidersStatus = jest.fn();
jest.mock('../../services/api', () => ({
  oauthApi: { getProvidersStatus: () => mockGetProvidersStatus() },
}));

import { ReconnectBanner } from '../ReconnectBanner';
import { CONNECTIONS_ROUTE } from '../../navigation/routes';

function renderBanner() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const view = render(
    <QueryClientProvider client={client}>
      <ReconnectBanner insetTop />
    </QueryClientProvider>,
  );
  const rerender = () =>
    view.rerender(
      <QueryClientProvider client={client}>
        <ReconnectBanner insetTop />
      </QueryClientProvider>,
    );
  return { ...view, rerenderBanner: rerender };
}

/** Wait for the status read to answer, so an absent banner is an answer, not a pending read. */
async function statusAnswered() {
  await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalled());
  await mockGetProvidersStatus.mock.results[0].value;
}

describe('ReconnectBanner', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockPathname = '/';
  });

  it('names each connected provider flagged needs_reauth, once, in the server order', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    const screen = renderBanner();

    // strava and sciotte are both "Strava"; the disconnected COROS is never named.
    expect(await screen.findByTestId('reconnect-banner-providers')).toHaveTextContent(
      'Reconnect Strava, Garmin to see your new activities.',
    );
    expect(screen.getByText('Reconnect needed')).toBeTruthy();
    expect(screen.getByTestId('reconnect-banner-action')).toHaveTextContent('Reconnect');
    expect(screen.queryByText(/COROS/)).toBeNull();
  });

  it('announces itself as an alert, with the button as its own focus stop', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    const screen = renderBanner();

    const message = await screen.findByTestId('reconnect-banner-message');
    expect(message.props.accessibilityRole).toBe('alert');
    expect(message.props.accessibilityLiveRegion).toBe('polite');
    const action = screen.getByTestId('reconnect-banner-action');
    expect(action.props.accessibilityRole).toBe('button');
    expect(action.props.accessibilityLabel).toBe('Reconnect');
  });

  it('leads to Connections', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    const screen = renderBanner();

    fireEvent.press(await screen.findByTestId('reconnect-banner-action'));
    expect(mockPush).toHaveBeenCalledWith(CONNECTIONS_ROUTE);
  });

  it('draws nothing while every connection is healthy', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_CONNECTED);
    const screen = renderBanner();

    await statusAnswered();
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();
  });

  it('draws nothing for a flagged provider that is not connected', async () => {
    mockGetProvidersStatus.mockResolvedValue({ providers: [PROVIDERS_RECONNECT.providers[4]] });
    const screen = renderBanner();

    await statusAnswered();
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();
  });

  it('draws nothing while the status read has not answered, or when it failed', async () => {
    mockGetProvidersStatus.mockRejectedValue(new Error('500'));
    const screen = renderBanner();

    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalled());
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();
  });

  it('stays off the Connections pane, and reads the status again when the athlete leaves it', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    mockPathname = '/connections';
    const screen = renderBanner();

    await statusAnswered();
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();

    // Reconnected on the pane; back on a tab, the banner asks again and goes.
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_CONNECTED);
    mockPathname = '/';
    screen.rerenderBanner();

    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(2));
    await mockGetProvidersStatus.mock.results[1].value;
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();
  });

  it('comes back when leaving the pane with a connection still flagged', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    mockPathname = '/connections';
    const screen = renderBanner();
    await statusAnswered();

    mockPathname = '/';
    screen.rerenderBanner();

    expect(await screen.findByTestId('reconnect-banner')).toBeTruthy();
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(2));
  });
});
