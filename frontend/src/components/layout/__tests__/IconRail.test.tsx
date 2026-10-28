// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the rail's account menu — the avatar opens Settings, Send feedback and Sign out under the athlete's name
// ABOUTME: Escape and a click outside close it, and a choice made in it closes it too (carnet#1235)

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { IconRail } from '../IconRail';

function renderRail() {
  const handlers = {
    onSelect: vi.fn(),
    onOpenSettings: vi.fn(),
    onSendFeedback: vi.fn(),
    onSignOut: vi.fn(),
    onHome: vi.fn(),
  };
  render(
    <IconRail
      tabs={[{ id: 'home', name: 'Home', icon: <span /> }]}
      activeTab="home"
      settingsActive={false}
      userInitial="A"
      userLabel="Alex Rider"
      {...handlers}
    />,
  );
  return handlers;
}

describe('IconRail account menu', () => {
  it('opens under the avatar with the athlete’s name and three choices', async () => {
    const user = userEvent.setup();
    renderRail();
    const toggle = screen.getByRole('button', { name: 'Account menu' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');

    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Alex Rider')).toBeInTheDocument();
    const settings = screen.getAllByRole('button', { name: 'Settings' });
    expect(settings).toHaveLength(2);
    expect(settings[1]).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Send feedback' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
  });

  it('opens the feedback thread and closes', async () => {
    const user = userEvent.setup();
    const { onSendFeedback } = renderRail();
    await user.click(screen.getByRole('button', { name: 'Account menu' }));
    await user.click(screen.getByRole('button', { name: 'Send feedback' }));
    expect(onSendFeedback).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('button', { name: 'Send feedback' })).not.toBeInTheDocument();
  });

  it('signs out from the menu', async () => {
    const user = userEvent.setup();
    const { onSignOut } = renderRail();
    await user.click(screen.getByRole('button', { name: 'Account menu' }));
    await user.click(screen.getByRole('button', { name: 'Sign out' }));
    expect(onSignOut).toHaveBeenCalledTimes(1);
  });

  it('closes on Escape and returns focus to the avatar', async () => {
    const user = userEvent.setup();
    renderRail();
    const toggle = screen.getByRole('button', { name: 'Account menu' });
    await user.click(toggle);
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('button', { name: 'Send feedback' })).not.toBeInTheDocument();
    expect(toggle).toHaveFocus();
  });

  it('closes on a click outside it', async () => {
    const user = userEvent.setup();
    renderRail();
    await user.click(screen.getByRole('button', { name: 'Account menu' }));
    await user.click(document.body);
    expect(screen.queryByRole('button', { name: 'Send feedback' })).not.toBeInTheDocument();
  });
});
