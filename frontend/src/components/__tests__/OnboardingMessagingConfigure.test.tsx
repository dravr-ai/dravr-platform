// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingMessagingConfigure — the QR/deep-link + poll-to-complete configure step
// ABOUTME: Verifies the QR renders for deep-link channels and the poll auto-advances via onLinked

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import OnboardingMessagingConfigure from '../OnboardingMessagingConfigure';

const { initLinkMock, listLinksMock } = vi.hoisted(() => ({
  initLinkMock: vi.fn(),
  listLinksMock: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  messagingLinkApi: { initLink: initLinkMock, listLinks: listLinksMock },
}));

function renderConfigure() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onLinked = vi.fn();
  const onSkip = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <OnboardingMessagingConfigure
        channel="telegram"
        displayName="Telegram"
        onLinked={onLinked}
        onSkip={onSkip}
      />
    </QueryClientProvider>,
  );
  return { onLinked, onSkip };
}

describe('OnboardingMessagingConfigure', () => {
  beforeEach(() => {
    initLinkMock.mockReset();
    listLinksMock.mockReset();
    initLinkMock.mockResolvedValue({
      channel: 'telegram',
      method: 'deep_link',
      code: 'abc',
      linking_url: 'https://t.me/DravrBot?start=abc',
      expires_at: '2026-07-08T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    });
    listLinksMock.mockResolvedValue([]);
  });

  it('renders the QR and the open button for a deep-link channel', async () => {
    renderConfigure();
    expect(
      await screen.findByRole('img', { name: /QR code to connect Telegram/i }),
    ).toBeInTheDocument();
    expect(screen.getByText('Open Telegram')).toBeInTheDocument();
  });

  it('auto-advances via onLinked once the channel link appears', async () => {
    listLinksMock.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'u1', display_name: null, linked_at: 'now' },
    ]);
    const { onLinked } = renderConfigure();
    await waitFor(() => expect(onLinked).toHaveBeenCalled());
  });

  describe('the poll is transient', () => {
    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('keeps asking while the link has not landed', async () => {
      renderConfigure();
      await waitFor(() => expect(listLinksMock).toHaveBeenCalledTimes(1));

      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS * 3);
      });
      // The athlete is watching this screen waiting for their phone; the poll
      // is what makes it advance.
      await waitFor(() => expect(listLinksMock.mock.calls.length).toBeGreaterThan(1));
    });

    it('stops the moment the link lands, instead of running for the life of the screen', async () => {
      renderConfigure();
      await waitFor(() => expect(listLinksMock).toHaveBeenCalledTimes(1));

      // The phone finishes: the next poll returns the link.
      listLinksMock.mockResolvedValue([
        { channel: 'telegram', channel_user_id: 'u1', display_name: null, linked_at: 'now' },
      ]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS);
      });
      await waitFor(() =>
        expect(listLinksMock.mock.results.length).toBeGreaterThanOrEqual(2),
      );

      // From here the answer cannot change, so nothing more is asked — even
      // if the athlete leaves the tab sitting there.
      const callsWhenLanded = listLinksMock.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS * 20);
      });
      expect(listLinksMock).toHaveBeenCalledTimes(callsWhenLanded);
    });
  });
});
