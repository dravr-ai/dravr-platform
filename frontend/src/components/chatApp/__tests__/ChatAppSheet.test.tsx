// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the chat-app sheet — choice then link, Use another app, and the close a landed link brings
// ABOUTME: A landed link records the offer complete and refreshes Getting started; closing the sheet answers nothing (carnet#1525)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ToastProvider } from '../../ui';
import { ChatAppSheet } from '../ChatAppSheet';

const api = vi.hoisted(() => ({
  setOnboardingStep: vi.fn(),
  getAvailableChannels: vi.fn(),
  listLinks: vi.fn(),
  initLink: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  userApi: { setOnboardingStep: api.setOnboardingStep },
  messagingLinkApi: {
    getAvailableChannels: api.getAvailableChannels,
    listLinks: api.listLinks,
    initLink: api.initLink,
  },
}));

const TELEGRAM = { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true };
const WHATSAPP = { channel: 'whatsapp', display_name: 'WhatsApp', method: 'deep_link', recommended: false };

function linkFor(channel: string) {
  return {
    channel,
    method: 'deep_link',
    code: 'abc',
    linking_url: `https://example.test/${channel}?start=abc`,
    expires_at: '2099-01-01T00:00:00Z',
    qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
  };
}

function renderSheet(props: { side?: 'bottom' | 'right'; initialChannel?: string } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <ChatAppSheet side={props.side ?? 'right'} initialChannel={props.initialChannel} onClose={onClose} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return { onClose, invalidate };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.getAvailableChannels.mockResolvedValue([TELEGRAM, WHATSAPP]);
  api.listLinks.mockResolvedValue([]);
  api.initLink.mockImplementation(async (channel: string) => linkFor(channel));
  api.setOnboardingStep.mockResolvedValue(undefined);
});

describe('ChatAppSheet', () => {
  it('opens on the choice when the workspace offers more than one app, then links the one picked', async () => {
    renderSheet();
    const whatsapp = await screen.findByTestId('chat-app-choose-whatsapp');
    expect(screen.getByRole('dialog', { name: 'Pick your app' })).toBeInTheDocument();
    await userEvent.click(whatsapp);
    expect(await screen.findByRole('dialog', { name: 'Connect WhatsApp' })).toBeInTheDocument();
    expect(await screen.findByText('Open WhatsApp')).toBeInTheDocument();
    expect(api.initLink).toHaveBeenCalledWith('whatsapp');
  });

  it('opens straight on the app the offer named, and Use another app goes back to the choice', async () => {
    renderSheet({ initialChannel: 'telegram' });
    expect(await screen.findByRole('dialog', { name: 'Connect Telegram' })).toBeInTheDocument();
    await userEvent.click(await screen.findByTestId('chat-app-use-another'));
    expect(await screen.findByTestId('chat-app-picker')).toBeInTheDocument();
  });

  it('goes straight to the only app, with no Use another app', async () => {
    api.getAvailableChannels.mockResolvedValue([TELEGRAM]);
    renderSheet();
    expect(await screen.findByText('Open Telegram')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-app-use-another')).toBeNull();
  });

  it('shows no QR on a phone: the sheet comes from the bottom there', async () => {
    renderSheet({ side: 'bottom', initialChannel: 'telegram' });
    expect(await screen.findByText('Open Telegram')).toBeInTheDocument();
    expect(screen.queryByRole('img', { name: /QR code/i })).toBeNull();
  });

  it('closes once the link lands, records the offer complete and refreshes Getting started', async () => {
    api.listLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'tg-1', display_name: null, linked_at: '2026-11-12T10:00:00Z' },
    ]);
    const { onClose, invalidate } = renderSheet({ initialChannel: 'telegram' });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('messaging_configure', 'complete');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['home', 'getting-started'] });
    expect(await screen.findByText('Telegram is set up')).toBeInTheDocument();
  });

  it('answers nothing when closed before linking', async () => {
    const { onClose } = renderSheet();
    await screen.findByTestId('chat-app-picker');
    await userEvent.click(screen.getByRole('button', { name: /close/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(api.setOnboardingStep).not.toHaveBeenCalled();
  });
});
