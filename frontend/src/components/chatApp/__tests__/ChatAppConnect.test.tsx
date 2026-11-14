// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for ChatAppConnect — the chat-app sheet's QR/deep-link + poll-to-complete link view
// ABOUTME: Verifies the QR renders for deep-link channels (never on a phone), the poll reports via onLinked, and Use another app

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import ChatAppConnect from '../ChatAppConnect';

const { initLinkMock, listLinksMock } = vi.hoisted(() => ({
  initLinkMock: vi.fn(),
  listLinksMock: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  messagingLinkApi: { initLink: initLinkMock, listLinks: listLinksMock },
}));

function renderConfigure({ showQr = true, withOthers = false }: { showQr?: boolean; withOthers?: boolean } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onLinked = vi.fn();
  const onUseAnother = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <ChatAppConnect
        channel="telegram"
        displayName="Telegram"
        showQr={showQr}
        onLinked={onLinked}
        onUseAnother={withOthers ? onUseAnother : undefined}
      />
    </QueryClientProvider>,
  );
  return { onLinked, onUseAnother };
}

describe('ChatAppConnect', () => {
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

  // No QR to scan from the same phone (carnet#1525): the tap is the only path.
  it('shows no QR on a phone, only the open button', async () => {
    renderConfigure({ showQr: false });
    expect(await screen.findByText('Open Telegram')).toBeInTheDocument();
    expect(screen.queryByRole('img', { name: /QR code/i })).not.toBeInTheDocument();
  });

  it('offers Use another app only when the workspace has another app', async () => {
    const { onUseAnother } = renderConfigure({ withOthers: true });
    await userEvent.click(await screen.findByTestId('chat-app-use-another'));
    expect(onUseAnother).toHaveBeenCalledTimes(1);
  });

  it('leaves Use another app out when this is the only app', async () => {
    renderConfigure();
    await screen.findByText('Open Telegram');
    expect(screen.queryByTestId('chat-app-use-another')).not.toBeInTheDocument();
  });

  it('tells a Telegram athlete to press Start', async () => {
    renderConfigure();
    const panel = await screen.findByTestId('channel-link-panel');
    expect(panel).toHaveTextContent('Then press Start to finish.');
  });

  // Found by the 2026-10-23 onboarding e2e on dev: the WhatsApp step said to
  // press Start, which WhatsApp does not have; its link opens a chat with the
  // code already typed.
  it('tells a WhatsApp athlete to send the pre-filled message', async () => {
    initLinkMock.mockResolvedValue({
      channel: 'whatsapp',
      method: 'deep_link',
      code: 'abc',
      linking_url: 'https://wa.me/15551562480?text=LINK%20abc',
      expires_at: '2026-07-08T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    });
    renderConfigure();
    const panel = await screen.findByTestId('channel-link-panel');
    expect(panel).toHaveTextContent('Then press Send to finish.');
    expect(panel).not.toHaveTextContent('Start');
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
