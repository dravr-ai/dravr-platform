// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for ChatAppPicker — the chat-app choice the chat-app sheet opens on
// ABOUTME: Verifies channels render, the recommended badge shows, select fires once, and a phone is never told to scan

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { AvailableChannel } from '@pierre/api-client';
import ChatAppPicker from '../ChatAppPicker';

const CHANNELS: AvailableChannel[] = [
  { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
  { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
];

describe('ChatAppPicker', () => {
  it('renders each channel and flags the recommended one', () => {
    render(<ChatAppPicker channels={CHANNELS} onSelect={vi.fn()} showQr />);
    expect(screen.getByText('Telegram')).toBeInTheDocument();
    expect(screen.getByText('Slack')).toBeInTheDocument();
    expect(screen.getByText('Recommended')).toBeInTheDocument();
  });

  it('calls onSelect with the channel id when a channel is chosen, and only once', async () => {
    const onSelect = vi.fn();
    render(<ChatAppPicker channels={CHANNELS} onSelect={onSelect} showQr />);
    await userEvent.click(screen.getByTestId('chat-app-choose-telegram'));
    await userEvent.click(screen.getByTestId('chat-app-choose-slack'));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith('telegram');
  });

  it('offers the QR on a wide screen', () => {
    render(<ChatAppPicker channels={CHANNELS} onSelect={vi.fn()} showQr />);
    expect(screen.getByTestId('chat-app-choose-telegram')).toHaveTextContent('Scan a QR or tap to open the app');
  });

  it('never tells a phone to scan a QR it would have to scan itself', () => {
    render(<ChatAppPicker channels={CHANNELS} onSelect={vi.fn()} showQr={false} />);
    expect(screen.getByTestId('chat-app-choose-telegram')).toHaveTextContent('Connect with one tap');
    expect(screen.queryByText(/QR/)).not.toBeInTheDocument();
  });
});
