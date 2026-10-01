// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for ChatAppLinks — the Settings → Messaging linked / available chat-app groups
// ABOUTME: Lists links, starts a link into the QR + deep-link panel, closes it when the link lands, unlinks

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, within, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import ChatAppLinks from '../ChatAppLinks';

const { listLinksMock, availableMock, initLinkMock, deleteLinkMock } = vi.hoisted(() => ({
  listLinksMock: vi.fn(),
  availableMock: vi.fn(),
  initLinkMock: vi.fn(),
  deleteLinkMock: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  messagingLinkApi: {
    listLinks: listLinksMock,
    getAvailableChannels: availableMock,
    initLink: initLinkMock,
    deleteLink: deleteLinkMock,
  },
}));

const TELEGRAM = { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true };
const SLACK = { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false };
const WHATSAPP = { channel: 'whatsapp', display_name: 'WhatsApp', method: 'deep_link', recommended: false };

const TELEGRAM_LINK = {
  channel: 'telegram',
  channel_user_id: 'tg-42',
  display_name: '@athlete',
  linked_at: '2026-09-01T10:00:00Z',
};

function renderLinks() {
  const client = new QueryClient({
    // gcTime Infinity schedules no cache-eviction timer, so a test can count
    // the timers the component itself leaves behind.
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { retry: false, gcTime: Infinity } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ChatAppLinks />
    </QueryClientProvider>,
  );
}

describe('ChatAppLinks', () => {
  beforeEach(() => {
    listLinksMock.mockReset();
    availableMock.mockReset();
    initLinkMock.mockReset();
    deleteLinkMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('lists the linked chat apps and offers only the unlinked ones', async () => {
    listLinksMock.mockResolvedValue([TELEGRAM_LINK]);
    availableMock.mockResolvedValue([TELEGRAM, SLACK, WHATSAPP]);
    renderLinks();

    const row = await screen.findByTestId('chat-app-link-telegram');
    expect(within(row).getByText('Telegram')).toBeInTheDocument();
    expect(within(row).getByText('@athlete')).toBeInTheDocument();
    expect(within(row).getByRole('button', { name: 'Unlink' })).toBeInTheDocument();

    const availableSection = screen.getByTestId('chat-app-available-section');
    const connects = within(availableSection).getAllByRole('button', { name: 'Connect' });
    expect(connects).toHaveLength(2);
    expect(within(availableSection).getByTestId('chat-app-add-slack')).toHaveTextContent('Slack');
    expect(within(availableSection).getByTestId('chat-app-add-whatsapp')).toHaveTextContent('WhatsApp');
    expect(within(availableSection).queryByTestId('chat-app-add-telegram')).not.toBeInTheDocument();
  });

  it('falls back to the channel user id when a link has no display name', async () => {
    listLinksMock.mockResolvedValue([{ ...TELEGRAM_LINK, display_name: null }]);
    availableMock.mockResolvedValue([TELEGRAM]);
    renderLinks();

    const row = await screen.findByTestId('chat-app-link-telegram');
    expect(within(row).getByText('tg-42')).toBeInTheDocument();
    expect(screen.getByTestId('chat-app-all-linked')).toHaveTextContent('Every available chat app is linked.');
  });

  it('says nothing is linked yet and points below when channels exist', async () => {
    listLinksMock.mockResolvedValue([]);
    availableMock.mockResolvedValue([TELEGRAM]);
    renderLinks();

    expect(await screen.findByTestId('chat-app-no-links')).toHaveTextContent(
      'No chat apps linked yet. Link one below to message your agent from it.',
    );
  });

  it('connecting opens the QR and deep link for the channel, and closes once the link lands', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    listLinksMock.mockResolvedValue([]);
    availableMock.mockResolvedValue([TELEGRAM]);
    initLinkMock.mockResolvedValue({
      channel: 'telegram',
      method: 'deep_link',
      code: 'abc123',
      linking_url: 'https://t.me/DravrBot?start=abc123',
      expires_at: '2030-01-01T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderLinks();

    await user.click(await screen.findByTestId('chat-app-connect-telegram'));

    expect(initLinkMock).toHaveBeenCalledWith('telegram');
    const panel = await screen.findByTestId('channel-link-panel');
    expect(within(panel).getByRole('img', { name: 'QR code to connect Telegram' })).toBeInTheDocument();
    const open = within(panel).getByRole('link');
    expect(open).toHaveAttribute('href', 'https://t.me/DravrBot?start=abc123');
    expect(open).toHaveTextContent('Open Telegram');
    expect(screen.getByRole('heading', { name: 'Connect Telegram' })).toBeInTheDocument();

    // The athlete finishes on the phone; the next poll sees the link.
    listLinksMock.mockResolvedValue([TELEGRAM_LINK]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS + 50);
    });

    await waitFor(() => expect(screen.queryByTestId('channel-link-panel')).not.toBeInTheDocument());
    expect(await screen.findByTestId('chat-app-link-telegram')).toBeInTheDocument();
    expect(screen.getByTestId('chat-app-all-linked')).toBeInTheDocument();
  });

  it('an OAuth channel gets a connect-with button instead of a QR', async () => {
    listLinksMock.mockResolvedValue([]);
    availableMock.mockResolvedValue([SLACK]);
    initLinkMock.mockResolvedValue({
      channel: 'slack',
      method: 'oauth',
      code: null,
      linking_url: 'https://slack.com/oauth/v2/authorize?state=xyz',
      expires_at: '2030-01-01T00:00:00Z',
      qr_svg: null,
    });
    const user = userEvent.setup();
    renderLinks();

    await user.click(await screen.findByTestId('chat-app-connect-slack'));

    const panel = await screen.findByTestId('channel-link-panel');
    expect(within(panel).queryByRole('img')).not.toBeInTheDocument();
    expect(within(panel).getByRole('link')).toHaveAttribute(
      'href',
      'https://slack.com/oauth/v2/authorize?state=xyz',
    );
    expect(within(panel).getByText('Connect with Slack')).toBeInTheDocument();
  });

  it('a failed link start says which channel could not start', async () => {
    listLinksMock.mockResolvedValue([]);
    availableMock.mockResolvedValue([TELEGRAM]);
    initLinkMock.mockRejectedValue(new Error('boom'));
    const user = userEvent.setup();
    renderLinks();

    await user.click(await screen.findByTestId('chat-app-connect-telegram'));

    expect(await screen.findByTestId('chat-app-links-action-error')).toHaveTextContent(
      "We couldn't start the Telegram connection just now.",
    );
    expect(screen.queryByTestId('channel-link-panel')).not.toBeInTheDocument();
  });

  it('unlink asks first, deletes the link, and the row moves back to Available', async () => {
    listLinksMock.mockResolvedValueOnce([TELEGRAM_LINK]).mockResolvedValue([]);
    availableMock.mockResolvedValue([TELEGRAM]);
    deleteLinkMock.mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderLinks();

    await user.click(await screen.findByTestId('chat-app-unlink-telegram'));

    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Unlink Telegram?')).toBeInTheDocument();
    expect(deleteLinkMock).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole('button', { name: 'Unlink' }));

    expect(deleteLinkMock).toHaveBeenCalledWith('telegram');
    await waitFor(() => expect(screen.queryByTestId('chat-app-link-telegram')).not.toBeInTheDocument());
    expect(await screen.findByTestId('chat-app-connect-telegram')).toBeInTheDocument();
    expect(listLinksMock).toHaveBeenCalledTimes(2);
  });

  it('a failed load shows the error and a retry, never an empty list', async () => {
    listLinksMock.mockRejectedValueOnce(new Error('down')).mockResolvedValue([TELEGRAM_LINK]);
    availableMock.mockResolvedValue([TELEGRAM]);
    const user = userEvent.setup();
    renderLinks();

    expect(await screen.findByTestId('chat-app-links-error')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-app-no-links')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('chat-app-links-retry'));

    expect(await screen.findByTestId('chat-app-link-telegram')).toBeInTheDocument();
  });

  it('a failed poll while the link panel is open keeps the panel, the QR and the lists', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    listLinksMock.mockResolvedValue([]);
    availableMock.mockResolvedValue([TELEGRAM, SLACK]);
    initLinkMock.mockResolvedValue({
      channel: 'telegram',
      method: 'deep_link',
      code: 'abc123',
      linking_url: 'https://t.me/DravrBot?start=abc123',
      expires_at: '2030-01-01T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderLinks();

    await user.click(await screen.findByTestId('chat-app-connect-telegram'));
    await screen.findByTestId('channel-link-panel');

    // One poll answers 502; the next one sees the link.
    listLinksMock.mockRejectedValueOnce(new Error('bad gateway'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS + 50);
    });

    expect(listLinksMock.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByTestId('chat-app-links-error')).not.toBeInTheDocument();
    const panel = screen.getByTestId('channel-link-panel');
    expect(within(panel).getByRole('img', { name: 'QR code to connect Telegram' })).toBeInTheDocument();
    expect(screen.getByTestId('chat-app-add-slack')).toBeInTheDocument();

    // The poll keeps going after the failure and still closes the panel on success.
    listLinksMock.mockResolvedValue([TELEGRAM_LINK]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS + 50);
    });
    await waitFor(() => expect(screen.queryByTestId('channel-link-panel')).not.toBeInTheDocument());
    expect(await screen.findByTestId('chat-app-link-telegram')).toBeInTheDocument();
  });

  it('a non-array available-channels body renders the empty state instead of throwing', async () => {
    listLinksMock.mockResolvedValue([]);
    availableMock.mockResolvedValue('<!doctype html><html><body>502</body></html>');
    renderLinks();

    expect(await screen.findByTestId('chat-app-none-configured')).toHaveTextContent(
      'No chat app is set up for your workspace yet, so there is nothing to link from here.',
    );
    expect(screen.getByTestId('chat-app-no-links')).toHaveTextContent(
      'No chat apps linked, and none are available on your workspace yet.',
    );
  });

  it('a non-array links body reads as nothing linked instead of throwing', async () => {
    listLinksMock.mockResolvedValue({ error: 'unexpected' });
    availableMock.mockResolvedValue([TELEGRAM]);
    renderLinks();

    expect(await screen.findByTestId('chat-app-no-links')).toHaveTextContent(
      'No chat apps linked yet. Link one below to message your agent from it.',
    );
    expect(screen.getByTestId('chat-app-connect-telegram')).toBeInTheDocument();
  });

  describe('after the link panel is closed', () => {
    const EXPIRES_IN_MS = 10 * 60_000;

    async function startAndCloseTelegramLink() {
      listLinksMock.mockResolvedValue([]);
      availableMock.mockResolvedValue([TELEGRAM]);
      initLinkMock.mockResolvedValue({
        channel: 'telegram',
        method: 'deep_link',
        code: 'abc123',
        linking_url: 'https://t.me/DravrBot?start=abc123',
        expires_at: new Date(Date.now() + EXPIRES_IN_MS).toISOString(),
        qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
      });
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      const view = renderLinks();
      await user.click(await screen.findByTestId('chat-app-connect-telegram'));
      await screen.findByTestId('channel-link-panel');
      await user.click(screen.getByRole('button', { name: 'Close modal' }));
      await waitFor(() => expect(screen.queryByTestId('channel-link-panel')).not.toBeInTheDocument());
      return view;
    }

    it('a link finished on the phone later still lands in Linked', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      await startAndCloseTelegramLink();
      expect(screen.getByTestId('chat-app-no-links')).toBeInTheDocument();

      // Two polls with the panel closed, then the athlete presses Start.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS * 2 + 50);
      });
      listLinksMock.mockResolvedValue([TELEGRAM_LINK]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS + 50);
      });

      const row = await screen.findByTestId('chat-app-link-telegram');
      expect(within(row).getByText('@athlete')).toBeInTheDocument();
      expect(screen.queryByTestId('channel-link-panel')).not.toBeInTheDocument();

      // Landed: the watch is over, so the pane stops asking.
      const calls = listLinksMock.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS * 3);
      });
      expect(listLinksMock).toHaveBeenCalledTimes(calls);
    });

    it('stops polling once the link code expires', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      await startAndCloseTelegramLink();

      await act(async () => {
        await vi.advanceTimersByTimeAsync(EXPIRES_IN_MS + CHANNEL_LINK_POLL_INTERVAL_MS);
      });
      const calls = listLinksMock.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(CHANNEL_LINK_POLL_INTERVAL_MS * 3);
      });
      expect(listLinksMock).toHaveBeenCalledTimes(calls);
    });

    it('unmounting clears the poll and the expiry timer', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const view = await startAndCloseTelegramLink();
      expect(vi.getTimerCount()).toBeGreaterThan(0);

      view.unmount();

      expect(vi.getTimerCount()).toBe(0);
      const calls = listLinksMock.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(EXPIRES_IN_MS + CHANNEL_LINK_POLL_INTERVAL_MS);
      });
      expect(listLinksMock).toHaveBeenCalledTimes(calls);
    });
  });
});
