// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingMessagingConfigure — the QR/deep-link + poll-to-complete configure step
// ABOUTME: Verifies the QR renders for deep-link channels and the poll auto-advances via onLinked

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
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
});
