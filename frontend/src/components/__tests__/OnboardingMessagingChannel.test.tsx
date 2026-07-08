// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingMessagingChannel — the messaging-app picker onboarding step
// ABOUTME: Verifies channels render, the recommended badge shows, and select/skip fire the callbacks

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { AvailableChannel } from '@pierre/api-client';
import OnboardingMessagingChannel from '../OnboardingMessagingChannel';

const CHANNELS: AvailableChannel[] = [
  { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
  { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
];

describe('OnboardingMessagingChannel', () => {
  it('renders each channel and flags the recommended one', () => {
    render(<OnboardingMessagingChannel channels={CHANNELS} onSelect={vi.fn()} onSkip={vi.fn()} />);
    expect(screen.getByText('Telegram')).toBeInTheDocument();
    expect(screen.getByText('Slack')).toBeInTheDocument();
    expect(screen.getByText('Recommended')).toBeInTheDocument();
  });

  it('calls onSelect with the channel id when a channel is chosen', async () => {
    const onSelect = vi.fn();
    render(<OnboardingMessagingChannel channels={CHANNELS} onSelect={onSelect} onSkip={vi.fn()} />);
    await userEvent.click(screen.getByText('Telegram'));
    expect(onSelect).toHaveBeenCalledWith('telegram');
  });

  it('calls onSkip when "Skip for now" is clicked', async () => {
    const onSkip = vi.fn();
    render(<OnboardingMessagingChannel channels={CHANNELS} onSelect={vi.fn()} onSkip={onSkip} />);
    await userEvent.click(screen.getByText('Skip for now'));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });
});
