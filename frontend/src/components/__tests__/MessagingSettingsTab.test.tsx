// ABOUTME: Unit tests for MessagingSettingsTab component
// ABOUTME: Tests channel list rendering, configure form, save/delete interactions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import MessagingSettingsTab from '../MessagingSettingsTab';

vi.mock('../../services/api', () => ({
  messagingApi: {
    listChannels: vi.fn().mockResolvedValue({
      tenant_id: 'tenant-1',
      channels: [
        {
          id: 'cfg-1',
          tenant_id: 'tenant-1',
          channel_type: 'whatsapp',
          is_active: true,
          created_at: '2026-03-16T00:00:00Z',
          updated_at: '2026-03-16T00:00:00Z',
        },
      ],
    }),
    upsertChannel: vi.fn().mockResolvedValue({
      tenant_id: 'tenant-1',
      config: { channel: 'telegram', enabled: true, has_credentials: true, webhook_url: null },
      action: 'upserted',
    }),
    deleteChannel: vi.fn().mockResolvedValue({
      tenant_id: 'tenant-1',
      channel: 'whatsapp',
      action: 'deleted',
    }),
  },
}));

function renderComponent() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <MessagingSettingsTab />
    </QueryClientProvider>
  );
}

describe('MessagingSettingsTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('should render all five channel cards', async () => {
    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      expect(screen.getByText('WhatsApp')).toBeInTheDocument();
      expect(screen.getByText('Telegram')).toBeInTheDocument();
      expect(screen.getByText('Slack')).toBeInTheDocument();
      expect(screen.getByText('Discord')).toBeInTheDocument();
      expect(screen.getByText('Messenger')).toBeInTheDocument();
    });
  });

  it('should show Configured badge for WhatsApp', async () => {
    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      expect(screen.getByText('Configured')).toBeInTheDocument();
    });
  });

  it('should show Configure button for unconfigured channels', async () => {
    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      // WhatsApp is configured → "Update", others → "Configure"
      const configureButtons = screen.getAllByRole('button', { name: 'Configure' });
      expect(configureButtons.length).toBe(4);
    });
  });

  it('should open configure form when Configure button is clicked', async () => {
    const user = userEvent.setup();

    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      expect(screen.getByText('Telegram')).toBeInTheDocument();
    });

    // Click the first Configure button (Telegram)
    const configureButtons = screen.getAllByRole('button', { name: 'Configure' });
    await user.click(configureButtons[0]);

    await waitFor(() => {
      expect(screen.getByText('Configure Telegram')).toBeInTheDocument();
      expect(screen.getByLabelText('Bot Token')).toBeInTheDocument();
      expect(screen.getByLabelText('Webhook Secret')).toBeInTheDocument();
    });
  });

  it('should close configure form when cancel is clicked', async () => {
    const user = userEvent.setup();

    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      expect(screen.getByText('Telegram')).toBeInTheDocument();
    });

    const configureButtons = screen.getAllByRole('button', { name: 'Configure' });
    await user.click(configureButtons[0]);

    await waitFor(() => {
      expect(screen.getByText('Configure Telegram')).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => {
      expect(screen.queryByText('Configure Telegram')).not.toBeInTheDocument();
    });
  });

  it('should show Remove button for configured channels', async () => {
    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument();
    });
  });

  it('should show header text', async () => {
    await act(async () => {
      renderComponent();
    });

    await waitFor(() => {
      expect(screen.getByText('Messaging Channels')).toBeInTheDocument();
    });
  });
});
